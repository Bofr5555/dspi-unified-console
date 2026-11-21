//! The calls the page's Worker makes, as JSON: `{"method": …, "params": …}`
//! in, `{"ok": true, "value": …}` or `{"ok": false, "code": …, "error": …}`
//! out. The shapes are the core's records as its serde makes them
//! (`dspi_core::DspState`, camelCase, which the C ABI's JSON replies carry
//! too); records without serde are written here with the same field names
//! in camelCase. `apps/web/src/core/protocol.ts` is the same contract in
//! TypeScript. Error codes are the C ABI's style (`read_only`).
//!
//! A module holds any number of sessions, each its own [`DspiCore`] behind a
//! handle: no device is "the" device. The page runs one session per Worker,
//! but nothing here relies on that.
//!
//! Every call goes through [`crate::guard`] first. No borrow of the session
//! table is held while the core runs, so a call that suspends leaves the
//! table usable.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use dspi_core::{DspiCore, DspiError, RtaRequest};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::guard::{self, Refusal};
// -- WP SW1 --
use crate::alink;
// -- WP WB6b --
use crate::curves;
// -- WP P0W --
use crate::history;
// -- WP WB4b --
use crate::link;
// -- WP WB6a --
use crate::presets;
// -- WP WB6c --
use crate::outputs;
// -- WP P1SN --
use crate::snapshots;
// -- WP P1LM --
use crate::limits;
// -- WP WB4a --
use crate::usb;
use crate::write;

/// An error reply: a stable code (protocol.ts lists them) and the core's
/// own wording.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub code: &'static str,
    pub message: String,
    // -- WP P1NT --
    /// The core error's notice key, for `noticeFor`; `None` for the binding's
    /// own refusals.
    pub notice_key: Option<String>,
}

impl CallError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            // -- WP P1NT --
            notice_key: None,
        }
    }
}

impl From<DspiError> for CallError {
    fn from(e: DspiError) -> Self {
        let code = match &e {
            DspiError::Disconnected => "not_open",
            DspiError::Transfer { .. } => "transfer",
            DspiError::Refused { .. } => "refused",
            DspiError::InvalidArgument { .. } => "invalid",
            DspiError::Unsupported => "unsupported",
            DspiError::ReadOnly => "read_only",
            DspiError::Locked => "locked",
            DspiError::Denied => "denied",
            DspiError::Link { code, .. } if code == "wrong_pin" => "wrong_pin",
            // -- WP WB7 -- (c6m N2) the page forgets the token it offered
            DspiError::Link { code, .. } if code == "wrong_hub" => "wrong_hub",
            DspiError::Link { .. } => "link",
            // -- WP P1TK --
            DspiError::ConfirmationRequired { .. } => "confirmation_required",
            // -- WP P1LM --
            DspiError::LimitExceeded { .. } => "limit_exceeded",
            // -- WP P1LK --
            DspiError::InstallerLock { .. } => "installer_lock",
            // -- WP P1DKFIX --
            DspiError::ImageRefused { .. } => "image_refused",
            DspiError::LimitsUnknown { .. } => "limits_unknown",
            // -- WP WB6I -- the core-voltage flow's word (CG14)
            DspiError::OvervoltWordRequired { .. } => "overvolt_word",
            _ => "internal",
        };
        // -- WP P1NT --
        let notice_key = Some(e.notice_key());
        Self {
            notice_key,
            ..Self::new(code, e.to_string())
        }
    }
}

type CallResult = Result<Value, CallError>;

// -- WP ROMODE --
/// A failure of session `h`'s with its `noticeKey` as the core answers it
/// now: `read_only.client` for a `read_only` while the client read-only mode
/// is on.
fn refine_for_session(h: u32, mut e: CallError) -> CallError {
    if e.notice_key.as_deref() != Some("read_only") {
        return e;
    }
    let core = SESSIONS.with_borrow(|s| s.get(h as usize)?.as_ref().map(|s| Arc::clone(&s.core)));
    if let Some(key) = core.and_then(|c| c.client_notice_key("read_only")) {
        e.notice_key = Some(key.to_owned());
    }
    e
}

/// `v` as JSON with every `f32` at its own precision: through serde_json's
/// text, which prints an `f32` as the shortest decimal that reads back the
/// same (`-12.3`), where `to_value` widens it to `f64` first
/// (`-12.300000190734863`).
pub fn tidy<T: serde::Serialize + ?Sized>(v: &T) -> Value {
    serde_json::to_string(v)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// One device session.
struct Session {
    core: Arc<DspiCore>,
    /// The open device is the core's demo: the demo's own simulations apply.
    demo: bool,
}

thread_local! {
    static SESSIONS: RefCell<Vec<Option<Session>>> = const { RefCell::new(Vec::new()) };
}

/// The reply for a call the guard did not let in.
pub fn refusal_reply(r: Refusal) -> Vec<u8> {
    let e = match r {
        Refusal::Busy => CallError::new("busy", "the core is already running a call"),
        Refusal::Dead => CallError::new(
            "dead",
            format!(
                "the core stopped after an internal error: {}",
                guard::panic_message()
            ),
        ),
    };
    reply(Err(e))
}

fn reply(r: CallResult) -> Vec<u8> {
    let v = match r {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(e) => {
            let mut v = json!({ "ok": false, "code": e.code, "error": e.message });
            // -- WP P1NT -- the core error's notice key, for `noticeFor`
            if let (Some(key), Some(fields)) = (e.notice_key, v.as_object_mut()) {
                fields.insert("noticeKey".to_owned(), Value::String(key));
            }
            v
        }
    };
    v.to_string().into_bytes()
}

/// A new session with no device; its handle.
pub fn session_new() -> Result<u32, Refusal> {
    let _in = guard::enter()?;
    Ok(SESSIONS.with_borrow_mut(|s| {
        let free = s.iter().position(Option::is_none);
        let session = Some(Session {
            core: DspiCore::new(),
            demo: false,
        });
        match free {
            Some(i) => {
                if let Some(slot) = s.get_mut(i) {
                    *slot = session;
                }
                u32::try_from(i).unwrap_or(u32::MAX)
            }
            None => {
                s.push(session);
                u32::try_from(s.len().saturating_sub(1)).unwrap_or(u32::MAX)
            }
        }
    }))
}

/// Close the session's device and forget it. False for an unknown handle.
pub fn session_free(h: u32) -> Result<bool, Refusal> {
    let _in = guard::enter()?;
    let taken = SESSIONS.with_borrow_mut(|s| s.get_mut(h as usize).and_then(Option::take));
    Ok(match taken {
        Some(s) => {
            s.core.disconnect();
            crate::autoeq_db::forget(h); // -- WP WB6K --
            true
        }
        None => false,
    })
}

#[derive(Deserialize)]
struct Request {
    method: String,
    #[serde(default)]
    params: Value,
}

/// Run one call on session `h`. The guard refuses a call while another runs;
/// a panic marks the module dead and answers an error.
pub fn call(h: u32, request: &[u8]) -> Vec<u8> {
    let entered = match guard::enter() {
        Ok(e) => e,
        Err(r) => return refusal_reply(r),
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        dispatch(h, request).map_err(|e| refine_for_session(h, e)) // -- WP ROMODE --
    }));
    drop(entered);
    match r {
        Ok(r) => reply(r),
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned());
            guard::mark_dead(&what);
            refusal_reply(Refusal::Dead)
        }
    }
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn dispatch(h: u32, request: &[u8]) -> CallResult {
    let req: Request = serde_json::from_slice(request)
        .map_err(|e| CallError::new("invalid", format!("not a call: {e}")))?;
    // The core out of the table, so no borrow is held while it runs.
    let (core, demo) = SESSIONS
        .with_borrow(|s| {
            s.get(h as usize)?
                .as_ref()
                .map(|s| (Arc::clone(&s.core), s.demo))
        })
        .ok_or_else(|| CallError::new("not_open", format!("no session {h}")))?;
    // -- WP SW1 -- the audio link (src/alink.rs)
    if let Some(r) = alink::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP SW3 -- the hub's audio-link system model (src/system.rs)
    if let Some(r) = crate::system::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P1LK -- the mode manifest and the installer's lock (src/modes.rs)
    if let Some(r) = crate::modes::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP HUBADMIN -- the hub's admin API (src/hub_admin.rs)
    if let Some(r) = crate::hub_admin::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P2CC1 -- the hub's own families (src/hub_server.rs)
    if let Some(r) = crate::hub_server::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P2CC2 -- the hub's admin families (src/hub_server_admin.rs)
    if let Some(r) = crate::hub_server_admin::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P2CC3 -- the hub's device and name writes (src/hub_server_manage.rs)
    if let Some(r) = crate::hub_server_manage::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P3CC -- the hub's fleet families (src/hub_server_fleet.rs)
    if let Some(r) = crate::hub_server_fleet::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5HUB -- the hub's measurement documents (src/hub_server_measure.rs)
    if let Some(r) = crate::hub_server_measure::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P1WB2 -- the DAC mute of the power-on page (src/power_on.rs)
    if let Some(r) = crate::power_on::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP ROMODE -- the client read-only mode (src/read_only.rs)
    if let Some(r) = crate::read_only::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP CG-SWS-a -- small stateless facts from the core (src/sws_a.rs)
    if let Some(r) = crate::sws_a::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP CG-SWS-b -- the pin map (src/pins.rs)
    if let Some(r) = crate::pins::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP WB6J -- the analyser's view, the signal generator, Stats for nerbs, the Interrupt Monitor (src/tools.rs)
    if let Some(r) = crate::tools::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP WB6L -- the control-surface editor and the quick strip (src/surfaces.rs)
    if let Some(r) = crate::surfaces::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP CLIP1 -- the channel clipboard (src/clipboard.rs)
    if let Some(r) = crate::clipboard::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP WB6I -- the Settings page (src/settings.rs)
    if let Some(r) = crate::settings::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5DOCK -- the measurement documents as text (src/measure_docs.rs)
    if let Some(r) = crate::measure_docs::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5LIVE -- the live response and direct adjustments (src/measure_live.rs)
    if let Some(r) = crate::measure_live::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5DEMO -- the demo's simulated microphone (src/measure_sim.rs)
    if let Some(r) = crate::measure_sim::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5TUNE -- the auto-tune (src/measure_tune.rs)
    if let Some(r) = crate::measure_tune::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP ATCORE -- the system measurement across units (src/measure_system.rs)
    if let Some(r) = crate::measure_system::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP INST0 -- Installation setup (src/installation.rs)
    if let Some(r) = crate::installation::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP PGCORE -- the protocol generation and raw paged requests (src/protocol.rs)
    if let Some(r) = crate::protocol::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP P5CURV -- the passive microphone RTA and a document's curves (src/measure_curv.rs)
    if let Some(r) = crate::measure_curv::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    // -- WP PGFWX -- the update target and the update check (src/firmware.rs)
    if let Some(r) = crate::firmware::dispatch(&core, &req.method, req.params.clone()) {
        return r;
    }
    match req.method.as_str() {
        "open" => open(h, &core, params(req.params)?),
        "state" => {
            need_open(&core)?;
            // -- WP WB4a -- (a USB device's serial) -- WP WB4b -- (a hub device's)
            Ok(link::with_serial(
                h,
                usb::with_serial(h, snapshot(&core, demo)),
            ))
        }
        "write" => {
            let w: write::WriteParams = params(req.params)?;
            let v = write::write(&core, &w)?;
            Ok(tidy(&v))
        }
        // -- WP CG36 --
        "core1Conflict" => write::core1_conflict(&core, &params(req.params)?),
        // -- WP P0W -- the change history (src/history.rs)
        "undo" => history::undo(&core),
        "redo" => history::redo(&core),
        "canUndo" | "canRedo" => Ok(history::can(&core)),
        "history" => Ok(history::history(&core, &params(req.params)?)),
        // -- WP QTGAP1 --
        "setDragging" => Ok(history::set_dragging(&core, &params(req.params)?)),
        // -- WP P1UG --
        "beginGroup" => Ok(history::begin_group(&core, params(req.params)?)),
        "endGroup" => Ok(history::end_group(&core)),
        "describe" => {
            #[derive(Deserialize)]
            struct P {
                path: String,
            }
            let p: P = params(req.params)?;
            write::describe(&core, &p.path)
        }
        "meters" => {
            // -- WP WB4b -- the meter frames that came over Link meanwhile
            core.link_service();
            meters(&core)
        }
        "poll" => {
            // -- WP WB4b -- the notifications that came over Link meanwhile
            core.link_service();
            poll(&core)
        }
        // -- WP P0M -- meter ballistics and the event log (src/calls.rs)
        "meterBallistics" => Ok(ballistics(&core)),
        "setMeterBallistics" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct P {
                hold_ms: u32,
                decay_per_s: f32,
                clip_hold_ms: u32,
            }
            let p: P = params(req.params)?;
            core.set_meter_ballistics(dspi_core::MeterBallistics {
                hold_ms: p.hold_ms,
                decay_per_s: p.decay_per_s,
                clip_hold_ms: p.clip_hold_ms,
            });
            Ok(ballistics(&core))
        }
        "resetMeterMax" => {
            core.reset_meter_max();
            Ok(Value::Null)
        }
        "eventLogRead" => event_log_read(&core, params(req.params)?),
        "eventLogExport" => {
            #[derive(Deserialize)]
            struct P {
                #[serde(default)]
                serial: String,
            }
            let p: P = params(req.params)?;
            Ok(json!(core.event_log_export(p.serial)))
        }
        "eventLogDevices" => Ok(json!(core.event_log_devices())),
        // -- WP WB8 -- the diagnostics bundle (src/diag.rs)
        "diagnostics" => crate::diag::diagnostics(&core, params(req.params)?),
        // -- WP WB8 -- a whole PEQ band in one write (src/band.rs)
        "filter" => crate::band::filter(&core, params(req.params)?),
        "rtaStart" => rta_start(&core, params(req.params)?),
        "rtaStop" => {
            core.rta_set_request(RtaRequest::default());
            Ok(Value::Null)
        }
        "rtaTick" => rta_tick(&core),
        "rta" => Ok(rta(&core)),
        "demo" => demo_action(&core, demo, params(req.params)?),
        // -- WP WB6a -- the preset flows `write` does not cover (src/presets.rs)
        "presetChanges" => presets::changes(&core),
        "presetCopy" => presets::copy(&core, params(req.params)?),
        "presetRevert" => presets::revert(&core),
        // -- WP P1TK -- with an optional `{token}`
        "factoryReset" => presets::factory_reset(&core, token_params(req.params)?),
        // -- WP P1DF -- the document diff and the import preview
        "configDiff" => presets::diff(params(req.params)?),
        "configPreview" => presets::preview(&core, params(req.params)?),
        // -- WP P1WB1 -- a plain import and the export, for the page's files
        "configImport" => presets::import(&core, params(req.params)?),
        "configExport" => presets::export(&core, params(req.params)?),
        // -- WP P1AFIX -- the crossfeed's custom pair, typed
        "crossfeedCustomPair" => presets::crossfeed_custom_pair(&core),
        // -- WP P1SN -- snapshots as documents (no files on the web)
        "snapshotCapture" => snapshots::capture(&core, params(req.params)?),
        "snapshotRecall" => snapshots::recall(&core, params(req.params)?),
        // -- WP P1AB -- A/B compare of snapshot documents (src/ab.rs)
        "abStart" => crate::ab::start(&core, params(req.params)?),
        "abToggle" => crate::ab::toggle(&core),
        "abDiff" => crate::ab::diff(&core),
        "abKeep" => crate::ab::keep(&core, params(req.params)?),
        "abState" => Ok(crate::ab::state(&core)),
        // -- WP P5SES -- the measurement session (src/measure.rs)
        "measurePreflight" => crate::measure::preflight(&core, params(req.params)?),
        "measureStart" => crate::measure::start(&core, params(req.params)?),
        "measureCapturePush" => crate::measure::push(&core, params(req.params)?),
        "measureTick" => Ok(crate::measure::tick(&core)),
        "measureNextPosition" => crate::measure::next_position(&core),
        "measureStop" => Ok(crate::measure::stop(&core)),
        "measureState" => Ok(crate::measure::state(&core)),
        "measureResult" => crate::measure::result(&core),
        "measureClear" => crate::measure::clear(&core),
        // -- WP P1LM -- limits profiles as documents (no files on the web)
        "limitsApply" => limits::apply(&core, params(req.params)?),
        "limitsProfile" => Ok(limits::profile(&core)),
        "limitsClear" => limits::clear(&core),
        "limitsStatus" => Ok(limits::status(&core)),
        // -- WP P1AQ -- AutoEQ preview and apply to a channel pair (src/autoeq.rs)
        "autoeqPreview" => crate::autoeq::preview(&core, params(req.params)?),
        "autoeqApply" => crate::autoeq::apply(&core, params(req.params)?),
        // -- WP WB6K -- the AutoEQ database the page keeps (src/autoeq_db.rs), filter files (src/filters.rs)
        "autoeqDatabaseLoad" => crate::autoeq_db::load(h, params(req.params)?),
        "autoeqDatabaseDownload" => crate::autoeq_db::download(h, params(req.params)?),
        "autoeqDatabaseInfo" => Ok(crate::autoeq_db::info(h)),
        "autoeqSearch" => crate::autoeq_db::search(h, params(req.params)?),
        "autoeqEntry" => crate::autoeq_db::entry(h, params(req.params)?),
        "autoeqFavourites" => Ok(crate::autoeq_db::favourites(h, params(req.params)?)),
        "autoeqFavouritesText" => Ok(crate::autoeq_db::favourites_text(params(req.params)?)),
        "filtersExport" => crate::filters::export(&core, params(req.params)?),
        "filtersInspect" => crate::filters::inspect(&core, params(req.params)?),
        "filtersImport" => crate::filters::import(&core, params(req.params)?),
        // -- WP P1SA -- safe apply with rollback (src/guarded.rs)
        "applyGuardedImport" => crate::guarded::import(&core, params(req.params)?),
        "applyGuardedSnapshot" => crate::guarded::snapshot(&core, params(req.params)?),
        "applyGuardBegin" => crate::guarded::begin(&core, params(req.params)?),
        "applyConfirm" => crate::guarded::confirm(&core),
        "applyRevert" => crate::guarded::revert(&core),
        "applyPoll" => Ok(crate::guarded::poll(&core)),
        "applyStatus" => Ok(crate::guarded::status(&core)),
        "applyGuardDefault" => Ok(crate::guarded::default_on(&core)),
        // -- WP P1OF -- offline editing and templates (src/offline.rs)
        "documentOpen" => document_open(h, &core, params(req.params)?),
        "documentInfo" => crate::offline::info(&core),
        "documentSave" => crate::offline::save(&core, params(req.params)?),
        "templateSave" => crate::offline::save_template(&core, params(req.params)?),
        "templateList" => crate::offline::list(),
        "templateText" => crate::offline::text(params(req.params)?),
        // -- WP WB6b -- the EQ graph's curves, the core's maths (src/curves.rs)
        "curveGrid" => Ok(curves::grid()),
        "curves" => curves::curves(params(req.params)?),
        // -- WP CG34 -- the filter types' names, the core's one table
        "filterTypeNames" => Ok(curves::type_names()),
        // -- WP WB6c -- the crossover's type, a crosspoint, the delay limit (src/outputs.rs)
        "crossoverType" => outputs::crossover_type(&core, params(req.params)?),
        "crosspoint" => outputs::crosspoint(&core, params(req.params)?),
        "outputLimits" => {
            need_open(&core)?;
            Ok(outputs::limits(&core))
        }
        // -- WP P1PR -- protect all outputs, the warning list, a linked pair's preamp
        "protectAllOutputs" => outputs::protect_all(&core, params(req.params)?),
        "outputsWithoutLimiter" => Ok(outputs::without_limiter(&core, params(req.params)?)),
        // -- WP WB6F -- the limiters' gain reduction per output in dB (`fetch_limiter_meter`,
        // as C's `dspi_fetch_limiter_meter`), for the status rail's health panel
        "limiterMeter" => {
            core.link_service();
            Ok(tidy(&core.fetch_limiter_meter()?))
        }
        "inputPairPreamp" => outputs::input_pair_preamp(&core, params(req.params)?),
        // -- WP WB6G -- the input sources' names, from the core's cache
        "inputSourceNames" => crate::inputs::source_names(&core),
        // -- WP WB6H -- the effect screens: the probe's feature names, the choices' words
        "features" => crate::effects::features(&core, params(req.params)?),
        "effectChoices" => Ok(crate::effects::effect_choices()),
        // -- WP PAIRBAND -- a linked input pair's band, one undo step
        "inputPairFilter" => crate::band::input_pair_filter(&core, params(req.params)?),
        "inputPairBandBypass" => crate::band::input_pair_band_bypass(&core, params(req.params)?),
        "close" => {
            core.disconnect();
            set_demo(h, false);
            // -- WP WB4a --
            usb::closed(h);
            // -- WP WB4b -- and leave the hub, giving back its session slot
            core.link_close();
            link::forget(h);
            Ok(Value::Null)
        }
        // -- WP WB4b -- Link (src/link.rs)
        "linkConnect" => link::connect(&core, params(req.params)?),
        "linkPair" => link::pair(&core, params(req.params)?),
        "linkDevices" => Ok(link::devices(&core)),
        "linkStatus" => Ok(link::status(&core)),
        "linkTick" => Ok(link::tick(&core)),
        // -- WP WB4a -- WebUSB (src/usb.rs)
        "usbPush" => usb::push(&core, params(req.params)?),
        "usbGone" => {
            set_demo(h, false);
            Ok(usb::gone(h, &core))
        }
        "usbIds" => Ok(usb::ids()),
        // -- WP P1NT -- the notice table (dspi_core::notice_text)
        "noticeFor" => {
            #[derive(Deserialize)]
            struct P {
                key: String,
                #[serde(default)]
                detail: String,
            }
            let p: P = params(req.params)?;
            Ok(tidy(&dspi_core::notice_for_key(p.key, p.detail)))
        }
        "noticeTexts" => Ok(tidy(&dspi_core::notice_texts())),
        // -- WP P1EFIX -- the limits editor's one range table
        "limitsFieldRanges" => Ok(tidy(&dspi_core::limits_field_ranges())),
        // -- WP P1TK -- confirmation tokens (dspi_core::confirm)
        "confirmToken" => {
            #[derive(Deserialize)]
            struct P {
                op: String,
            }
            let p: P = params(req.params)?;
            let op = dspi_core::ConfirmOp::from_key(&p.op).ok_or_else(|| {
                CallError::new(
                    "invalid",
                    format!("{} is not an operation that is confirmed", p.op),
                )
            })?;
            Ok(json!(core.confirm_token(op)))
        }
        // -- WP P1TZ -- tokens are always required: requireConfirmation and
        // confirmationRequired are gone (an unknown method now).
        #[cfg(any(test, feature = "test-hooks"))]
        "test.sleep" => {
            #[derive(Deserialize)]
            struct P {
                ms: u64,
            }
            let p: P = params(req.params)?;
            dspi_transport::clock::sleep(std::time::Duration::from_millis(p.ms));
            Ok(json!(p.ms))
        }
        #[cfg(any(test, feature = "test-hooks"))]
        "test.panic" => panic!("test.panic was called"),
        other => Err(CallError::new(
            "unknown_method",
            format!("unknown method {other}"),
        )),
    }
}

// -- WP P1TK --
/// The parameters of a call that takes only an optional token: null (none)
/// or `{token}`.
fn token_params(v: Value) -> Result<presets::TokenParams, CallError> {
    if v.is_null() {
        Ok(presets::TokenParams::default())
    } else {
        params(v)
    }
}

fn need_open(core: &DspiCore) -> Result<(), CallError> {
    if core.is_connected() {
        Ok(())
    } else {
        Err(CallError::new(
            "not_open",
            "No DSPi is open in this session",
        ))
    }
}

fn set_demo(h: u32, demo: bool) {
    SESSIONS.with_borrow_mut(|s| {
        if let Some(Some(s)) = s.get_mut(h as usize) {
            s.demo = demo;
        }
    });
}

/// `open`'s parameters: protocol.ts `DeviceTarget`.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Target {
    Demo {
        #[serde(default, rename = "readOnly")]
        read_only: bool,
        // -- WP SW1 -- a unit receiving the audio link
        #[serde(default, rename = "alink")]
        alink: bool,
        // -- WP AUSB1 -- an alink USB-direct unit
        #[serde(default, rename = "usbDirect")]
        usb_direct: bool,
    },
    // -- WP WB4a --
    Usb(usb::UsbTarget),
    // -- WP WB4b --
    Hub(link::HubTarget),
}

// -- WP P1OF --
/// `documentOpen`: the document as the session's device, replacing whatever it
/// had open, as `open` does. The core reads it (`fetch_all`) itself. The reply
/// is `open`'s snapshot with the core's description as `document`; `device.demo`
/// is false, since it is the document, not the demo, that is open.
fn document_open(h: u32, core: &DspiCore, p: crate::offline::OpenParams) -> CallResult {
    let document = crate::offline::open(core, p)?;
    set_demo(h, false);
    usb::closed(h);
    link::forget(h);
    let mut snap = snapshot(core, false);
    if let Some(m) = snap.as_object_mut() {
        m.insert("document".into(), document);
    }
    Ok(snap)
}

/// The demo in an older wire format: the core reads it and refuses writes.
const READ_ONLY_DEMO_WIRE: u8 = 31;

fn open(h: u32, core: &DspiCore, target: Target) -> CallResult {
    match target {
        Target::Demo {
            read_only,
            alink,
            usb_direct,
        } => {
            if read_only {
                core.open_demo_wire(READ_ONLY_DEMO_WIRE)?;
            } else if usb_direct {
                // -- WP AUSB1 --
                core.open_demo_usb_direct()?;
            } else if alink {
                // -- WP SW1 --
                core.open_demo_alink()?;
            } else {
                core.open_demo()?;
            }
            set_demo(h, true);
            // -- WP WB4b -- (c6g N1) whatever this session had on USB or a hub is not it
            usb::closed(h);
            link::forget(h);
            // -- WP WB6a -- -- WP WB6b -- (c6p WB6a-2) a failed read leaves no demo flag
            if let Err(e) = presets::opened(core) {
                set_demo(h, false);
                return Err(e);
            }
            Ok(snapshot(core, true))
        }
        // -- WP WB4a --
        Target::Usb(t) => {
            link::forget(h);
            usb::open(h, core, t)?;
            set_demo(h, false);
            usb_opened(h, core)
        }
        // -- WP WB4b --
        Target::Hub(t) => {
            usb::closed(h);
            link::open(h, core, &t)?;
            set_demo(h, false);
            Ok(link::with_serial(h, snapshot(core, false)))
        }
    }
}

// -- WP WB6a -- -- WP WB6b --
/// The rest of a USB open once the device is claimed: the reads (as the
/// demo's), then the snapshot. (c6p WB6a-2) When they fail the device is
/// closed and its claim forgotten too, as `close` does.
pub(crate) fn usb_opened(h: u32, core: &DspiCore) -> CallResult {
    if let Err(e) = presets::opened(core) {
        usb::closed(h);
        return Err(e);
    }
    Ok(usb::with_serial(h, snapshot(core, false)))
}

// -- WP WB4a --
/// Session `h`'s core, for the USB tests.
#[cfg(test)]
pub(crate) fn core_of(h: u32) -> Option<Arc<DspiCore>> {
    SESSIONS.with_borrow(|s| s.get(h as usize)?.as_ref().map(|s| Arc::clone(&s.core)))
}

/// What the page shows of a device: who it is (`device`) and the core's
/// mirror (`state`, `DspState` without its raw image `bulk_raw`).
fn snapshot(core: &DspiCore, demo: bool) -> Value {
    let s = core.get_state();
    let read_only = s.format_version != dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8;
    let name = core
        .cached_device_name()
        .ok()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "DSPi".to_owned());
    let mut firmware = format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch);
    if s.fw_beta != 0 {
        firmware.push_str(&format!("-beta{}", s.fw_beta));
    }
    let platform = match s.platform_id {
        0 => "RP2040",
        1 => "RP2350",
        _ => "unknown",
    };
    let serial = if demo { demo_serial() } else { String::new() };
    let mut state = tidy(&s);
    if let Some(m) = state.as_object_mut() {
        m.remove("bulkRaw");
    }
    json!({
        "device": {
            "name": name,
            "serial": serial,
            "firmware": firmware,
            "platform": platform,
            "demo": demo,
            "readOnly": read_only,
            "readOnlyReason": if read_only { DspiError::ReadOnly.to_string() } else { String::new() },
        },
        "state": state,
        // -- BUG1 -- what each output is called and how the dashboard pairs
        // them, worked out once per snapshot (src/output_labels.rs in the core)
        "outputLabels": tidy(&core.output_labels()),
        "outputCards": tidy(&core.output_cards()),
    })
}

/// The serial the core's demo reports.
fn demo_serial() -> String {
    thread_local! {
        static SERIAL: String = {
            use dspi_transport::Transport;
            dspi_transport::FakeDevice::demo().descriptor().serial.clone()
        };
    }
    SERIAL.with(Clone::clone)
}

/// `meters`: `dspi_core::SystemStatus`, its field names in camelCase.
fn meters(core: &DspiCore) -> CallResult {
    let st = core.fetch_status()?;
    Ok(json!({
        "peaks": tidy(&st.peaks),
        "cpu0": st.cpu0,
        "cpu1": st.cpu1,
        "clipFlags": st.clip_flags,
        "activeInputChannels": st.active_input_channels,
        "numChannels": st.num_channels,
        "stale": st.stale,
        "ageMs": st.age_ms,
        // -- WP P0M -- the core's ballistics
        "peakHold": tidy(&st.peak_hold),
        "peakMax": tidy(&st.peak_max),
        "clipHold": st.clip_hold,
    }))
}

// -- WP P0M --
/// `meterBallistics`: the timing, camelCase.
fn ballistics(core: &DspiCore) -> Value {
    let b = core.meter_ballistics();
    json!({ "holdMs": b.hold_ms, "decayPerS": b.decay_per_s, "clipHoldMs": b.clip_hold_ms })
}

/// `eventLogRead`'s parameters: `serial` "" or absent for the open device.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventLogParams {
    #[serde(default)]
    serial: String,
    #[serde(default)]
    after_id: u64,
    #[serde(default = "event_limit")]
    limit: u32,
}

fn event_limit() -> u32 {
    500
}

/// `eventLogRead`: the records in the event log's file format, oldest first,
/// and the id to pass next. The web keeps the event log in memory (no files).
fn event_log_read(core: &DspiCore, p: EventLogParams) -> CallResult {
    let page = core.event_log_page(p.serial, p.after_id, p.limit);
    let records = serde_json::to_value(&page.records)
        .map_err(|e| CallError::new("internal", e.to_string()))?;
    Ok(json!({ "records": records, "lastId": page.last_id, "skipped": page.skipped }))
}

/// `poll`: the Worker's notification turn. What changed (`NOTIFY_*` flags),
/// when a failed re-read is due again, and whether the device is still there.
fn poll(core: &DspiCore) -> CallResult {
    if !core.is_connected() {
        return Ok(json!({ "flags": 0, "retryMs": null, "connected": false }));
    }
    let r = core.process_notifications();
    let retry = core
        .notify_retry_due_in()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Ok(json!({
        "flags": r.flags,
        "retryMs": retry,
        "connected": core.is_connected(),
    }))
}

/// `rta_start`'s parameters: the fields of `RtaRequest` the page sets.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RtaParams {
    tap: u8,
    mask: u16,
    #[serde(default)]
    wants_bins: bool,
    #[serde(default)]
    fft_order: u8,
    #[serde(default)]
    avg_ms: u16,
    #[serde(default)]
    peak_decay_db_s: u8,
}

fn rta_start(core: &DspiCore, p: RtaParams) -> CallResult {
    need_open(core)?;
    core.rta_start();
    core.rta_set_request(RtaRequest {
        tap: p.tap,
        mask: p.mask,
        wants_bins: p.wants_bins,
        fft_order: p.fft_order,
        avg_ms: p.avg_ms,
        peak_decay_db_s: p.peak_decay_db_s,
        ..RtaRequest::default()
    });
    Ok(Value::Null)
}

/// `rta_tick`: one turn of the analyser, as its thread takes natively. The
/// wait before the next, in ms, or null when there is nothing to tick.
fn rta_tick(core: &DspiCore) -> CallResult {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let wait = core
        .rta_tick()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    // Natively the core's own worker thread takes the turns.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let wait: Option<u64> = {
        let _ = core;
        None
    };
    Ok(json!({ "waitMs": wait }))
}

/// `rta`: the analyser's caps and latest picture, its field names in camelCase.
fn rta(core: &DspiCore) -> Value {
    let c = core.rta_get_caps();
    let s = core.rta_get_snapshot();
    let count = usize::from(s.count);
    let frames: Vec<Value> = s
        .frames
        .iter()
        .take(count)
        .map(|f| {
            json!({
                "channel": f.channel,
                "nBands": f.n_bands,
                "silent": f.silent,
                "avgDb": tidy(&f.avg_db),
                "peakDb": tidy(&f.peak_db),
            })
        })
        .collect();
    json!({
        "caps": {
            "supported": c.supported,
            "inputChannels": c.input_channels,
            "outputChannels": c.output_channels,
            "orderMin": c.order_min,
            "orderMax": c.order_max,
            "orderDefault": c.order_default,
            "bassBands": c.bass_bands,
        },
        "snapshot": {
            "version": s.version,
            "tap": s.tap,
            "count": s.count,
            "frames": frames,
            "populated": s.populated,
            "running": s.running,
            "framesPerS": s.frames_per_s,
            "sampleRate": s.sample_rate,
            "fftOrder": s.fft_order,
            "configRejected": s.config_rejected,
            "refreshIntervalS": tidy(&s.refresh_interval_s),
        },
    })
}

/// `demo`'s parameters: the core's own demo simulations.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum DemoAction {
    /// Another host renames the device: it arrives as a notification.
    RenameElsewhere { name: String },
    // -- WP WB8 --
    /// The device clips on the channels in `mask`: the next meter read
    /// shows it and the event log records it.
    Clip { mask: u32 },
}

/// `demo_rename_elsewhere`'s target for the device name (input sources are
/// 0..).
const DEVICE_NAME_TARGET: u8 = 0xFF;

fn demo_action(core: &DspiCore, demo: bool, a: DemoAction) -> CallResult {
    if !demo {
        return Err(CallError::new("refused", "only the demo DSPi does that"));
    }
    match a {
        DemoAction::RenameElsewhere { name } => {
            core.demo_rename_elsewhere(DEVICE_NAME_TARGET, name)?;
        }
        DemoAction::Clip { mask } => core.demo_clip(mask)?,
    }
    Ok(Value::Null)
}

// -- WP P1NT --
#[cfg(test)]
mod tests_wp_p1nt;
// -- WP P1TK --
#[cfg(test)]
mod tests_wp_p1tk;
// -- WP WB6F --
#[cfg(test)]
mod tests_wp_wb6f;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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

    fn code(r: &Value) -> &str {
        r["code"].as_str().unwrap_or("")
    }

    // -- WP P0W --
    #[test]
    fn wp_p0w_undo_redo_and_history_through_the_worker_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let can = ok(h, "canUndo", Value::Null);
        assert_eq!(can, json!({ "canUndo": false, "canRedo": false }));
        ok(
            h,
            "write",
            json!({ "path": "out.gain", "indices": [0], "value": { "float": -6.5 } }),
        );
        let flags = ok(h, "poll", Value::Null)["flags"].as_u64().unwrap();
        assert_ne!(flags & u64::from(dspi_core::consts::NOTIFY_HISTORY), 0);
        let hist = ok(h, "history", json!({ "n": 5 }));
        assert_eq!(hist[0]["path"], "out.gain");
        assert_eq!(hist[0]["after"], json!({ "float": -6.5 }));
        assert_eq!(hist[0]["source"], "local");
        let undone = ok(h, "undo", Value::Null);
        assert_eq!(undone["entry"]["path"], "out.gain");
        assert_eq!(ok(h, "canRedo", Value::Null)["canRedo"], true);
        let redone = ok(h, "redo", Value::Null);
        assert_eq!(redone["entry"]["after"], json!({ "float": -6.5 }));
        assert_eq!(ok(h, "history", json!({ "n": 1 }))[0]["source"], "redo");
        ok(h, "close", Value::Null);
        assert_eq!(code(&run(h, "undo", Value::Null)), "not_open");
        session_free(h).unwrap();
    }

    // -- WP QTGAP1 --
    #[test]
    fn wp_qtgap1_only_a_marked_drag_joins_one_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let gain =
            |db: f64| json!({ "path": "out.gain", "indices": [0], "value": { "float": db } });
        ok(h, "write", gain(-1.0));
        ok(h, "write", gain(-2.0));
        ok(h, "setDragging", json!({ "dragging": true }));
        ok(h, "write", gain(-3.0));
        ok(h, "write", gain(-4.0));
        ok(h, "setDragging", json!({ "dragging": false }));
        assert_eq!(
            ok(h, "history", json!({ "n": 10 }))
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(code(&run(h, "setDragging", Value::Null)), "invalid");
        session_free(h).unwrap();
    }

    // -- WP P1UG --
    /// `beginGroup` and `endGroup`: the writes between them are one step
    /// with the label; one undo takes both back.
    #[test]
    fn wp_p1ug_an_undo_group_through_the_worker_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let start = core_of(h).unwrap().get_state().output_gain_db;
        let gain = |o: u8, db: f64| json!({ "path": "out.gain", "indices": [o], "value": { "float": db } });
        ok(h, "beginGroup", json!({ "label": "Linked pair" }));
        ok(h, "write", gain(0, -5.0));
        ok(h, "write", gain(1, -5.0));
        ok(h, "endGroup", Value::Null);
        let hist = ok(h, "history", json!({ "n": 10 }));
        assert_eq!(hist.as_array().unwrap().len(), 1, "{hist}");
        assert_eq!(hist[0]["label"], "Linked pair");
        let undone = ok(h, "undo", Value::Null);
        assert_eq!(undone["entry"]["label"], "Linked pair");
        assert_eq!(core_of(h).unwrap().get_state().output_gain_db, start);
        assert_eq!(ok(h, "canUndo", Value::Null)["canUndo"], false);
        assert_eq!(code(&run(h, "beginGroup", Value::Null)), "invalid");
        session_free(h).unwrap();
    }

    // -- WP P0W2 --
    /// A graph drag of one band through the worker calls, frequency alone
    /// and frequency with gain mixed, is one step; undo puts the band back.
    #[test]
    fn wp_p0w2_a_mixed_axis_band_drag_is_one_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let band = || core_of(h).unwrap().get_state().filters[0][1];
        let start = band();
        let field = |path: &str, v: f32| json!({ "path": path, "indices": [0, 1], "value": { "float": v } });
        ok(h, "setDragging", json!({ "dragging": true }));
        ok(h, "write", field("eq.freq", start.freq + 10.0));
        ok(h, "write", field("eq.freq", start.freq + 30.0));
        ok(h, "write", field("eq.gain", start.gain + 1.0));
        ok(h, "write", field("eq.freq", start.freq + 35.0));
        ok(h, "setDragging", json!({ "dragging": false }));
        let hist = ok(h, "history", json!({ "n": 10 }));
        assert_eq!(hist.as_array().unwrap().len(), 1, "{hist}");
        assert_eq!(hist[0]["path"], "eq.band");
        ok(h, "undo", Value::Null);
        assert_eq!(band(), start);
        assert_eq!(ok(h, "canUndo", Value::Null)["canUndo"], false);
        session_free(h).unwrap();
    }

    // -- WP P0W2 (fixup p0w2b, F3) --
    /// A drag mark left set when the page closes the device joins nothing
    /// after it opens one again.
    #[test]
    fn wp_p0w2b_f3_a_drag_mark_ends_with_the_session_device() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let gain =
            |db: f64| json!({ "path": "out.gain", "indices": [0], "value": { "float": db } });
        ok(h, "setDragging", json!({ "dragging": true }));
        ok(h, "write", gain(-1.0));
        ok(h, "close", Value::Null);
        ok(h, "open", json!({ "kind": "demo" }));
        ok(h, "write", gain(-2.0));
        ok(h, "write", gain(-3.0));
        let hist = ok(h, "history", json!({ "n": 10 }));
        assert_eq!(hist.as_array().unwrap().len(), 2, "{hist}");
        session_free(h).unwrap();
    }

    // -- WP P0W2 (O1) --
    /// The web's multi-selection graph drag: two bands sent whole with
    /// `filter` every tick, four ticks, mixed axes, is one step; one undo
    /// puts both bands back.
    #[test]
    fn wp_p0w2_o1_a_multi_band_web_drag_is_one_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let band = |b: usize| core_of(h).unwrap().get_state().filters[0][b];
        let (s1, s2) = (band(1), band(2));
        ok(h, "setDragging", json!({ "dragging": true }));
        for (df, dg) in [(10.0f32, 0.0f32), (20.0, 1.0), (0.0, 1.5), (5.0, 0.0)] {
            for b in [1u8, 2] {
                let p = band(usize::from(b));
                ok(
                    h,
                    "filter",
                    json!({ "channel": 0, "band": b, "filterType": p.filter_type,
                            "freq": p.freq + df, "q": p.q, "gain": p.gain + dg,
                            "qp": p.qp, "bypass": p.bypass }),
                );
            }
        }
        ok(h, "setDragging", json!({ "dragging": false }));
        let hist = ok(h, "history", json!({ "n": 10 }));
        assert_eq!(hist.as_array().unwrap().len(), 1, "{hist}");
        assert_ne!((band(1), band(2)), (s1, s2));
        ok(h, "undo", Value::Null);
        assert_eq!((band(1), band(2)), (s1, s2));
        assert_eq!(ok(h, "canUndo", Value::Null)["canUndo"], false);
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_two_demo_sessions_connect_write_with_read_back_and_stay_apart() {
        guard::reset_for_test();
        let a = session_new().unwrap();
        let b = session_new().unwrap();
        assert_ne!(a, b);
        for h in [a, b] {
            let snap = ok(h, "open", json!({ "kind": "demo" }));
            assert_eq!(snap["device"]["demo"], true);
            assert_eq!(snap["device"]["readOnly"], false);
            assert!(!snap["device"]["serial"].as_str().unwrap().is_empty());
            assert!(snap["state"]["userVolumeDb"].is_number());
            assert!(snap["state"].get("bulkRaw").is_none());
        }
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -17.5 } }),
        );
        assert_eq!(v, json!({ "float": -17.5 }));
        // f32 at its own precision, not widened to f64's digits.
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -12.3 } }),
        );
        assert_eq!(v.to_string(), r#"{"float":-12.3}"#);
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -17.5 } }),
        );
        assert_eq!(v, json!({ "float": -17.5 }));
        assert_eq!(ok(a, "state", Value::Null)["state"]["userVolumeDb"], -17.5);
        assert_ne!(ok(b, "state", Value::Null)["state"]["userVolumeDb"], -17.5);

        let g = ok(
            a,
            "write",
            json!({ "path": "out.gain", "indices": [1], "value": { "float": -3.0 } }),
        );
        assert_eq!(g, json!({ "float": -3.0 }));

        // -- c6e W3-1 -- `pre` trims the one input asked for, `pre.all` every input.
        let p = ok(
            a,
            "write",
            json!({ "path": "pre", "indices": [1], "value": { "float": -4.0 } }),
        );
        assert_eq!(p, json!({ "float": -4.0 }));
        let st = ok(a, "state", Value::Null);
        assert_eq!(st["state"]["inputPreampDb"][1], -4.0);
        assert_eq!(st["state"]["inputPreampDb"][0], 0.0, "only input 1 changed");
        let r = run(
            a,
            "write",
            json!({ "path": "pre", "indices": [200], "value": { "float": -4.0 } }),
        );
        assert!(
            !code(&r).is_empty(),
            "an input the device does not have is refused: {r}"
        );
        let all = ok(
            a,
            "write",
            json!({ "path": "pre.all", "value": { "float": -2.0 } }),
        );
        assert_eq!(all, json!({ "float": -2.0 }));

        let r = run(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": 3.0 } }),
        );
        assert_eq!(code(&r), "out_of_range");
        assert_eq!(r["error"], "Volume must be between -60 and 0 dB");
        let r = run(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "bool": true } }),
        );
        assert_eq!(code(&r), "invalid");
        let r = run(
            a,
            "write",
            json!({ "path": "out.gain", "value": { "float": -3.0 } }),
        );
        assert_eq!(code(&r), "invalid", "out.gain needs its output");
        let r = run(
            a,
            "write",
            json!({ "path": "no.such", "value": { "float": 0.0 } }),
        );
        assert_eq!(code(&r), "unknown_param");

        ok(a, "close", Value::Null);
        assert_eq!(code(&run(a, "state", Value::Null)), "not_open");
        for h in [a, b] {
            assert!(session_free(h).unwrap());
        }
        assert_eq!(code(&run(a, "state", Value::Null)), "not_open");
    }

    #[test]
    fn wb3_an_older_firmware_demo_is_read_only_in_the_cores_words() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let snap = ok(h, "open", json!({ "kind": "demo", "readOnly": true }));
        assert_eq!(snap["device"]["readOnly"], true);
        assert_eq!(
            snap["device"]["readOnlyReason"],
            "the connected firmware is read-only in this version"
        );
        let r = run(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -10.0 } }),
        );
        assert_eq!(code(&r), "read_only");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_usb_and_hubs_are_not_offered_yet_and_unknown_methods_say_so() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // -- WP WB4a -- USB is offered now; without a chosen device it says so.
        assert_eq!(code(&run(h, "open", json!({ "kind": "usb" }))), "invalid");
        // -- WP WB4b -- hubs are offered now: a device must be chosen, on a connected hub.
        assert_eq!(
            code(&run(
                h,
                "open",
                json!({ "kind": "hub", "url": "ws://h:1", "deviceId": "" })
            )),
            "invalid"
        );
        assert_eq!(
            code(&run(
                h,
                "open",
                json!({ "kind": "hub", "url": "ws://h:1", "deviceId": "X" })
            )),
            "not_open"
        );
        assert_eq!(
            code(&run(h, "simulate", json!({ "what": "link-lost" }))),
            "unknown_method"
        );
        assert_eq!(code(&run(9999, "state", Value::Null)), "not_open");
        let out = call(h, b"not json");
        assert_eq!(code(&serde_json::from_slice(&out).unwrap()), "invalid");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_a_foreign_rename_arrives_through_poll() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        ok(
            h,
            "demo",
            json!({ "action": "renameElsewhere", "name": "Elsewhere" }),
        );
        let until = Instant::now() + Duration::from_secs(5);
        let mut flags = 0;
        while flags & dspi_core::consts::NOTIFY_NAMES == 0 && Instant::now() < until {
            flags |= ok(h, "poll", Value::Null)["flags"].as_u64().unwrap() as u32;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(flags & dspi_core::consts::NOTIFY_NAMES, 0);
        assert_eq!(ok(h, "state", Value::Null)["device"]["name"], "Elsewhere");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_meters_and_the_analyser_answer_in_the_cores_shapes() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let m = ok(h, "meters", Value::Null);
        assert!(!m["peaks"].as_array().unwrap().is_empty());
        ok(h, "rtaStart", json!({ "tap": 1, "mask": 3 }));
        let until = Instant::now() + Duration::from_secs(10);
        let mut count = 0;
        while count == 0 && Instant::now() < until {
            count = ok(h, "rta", Value::Null)["snapshot"]["count"]
                .as_u64()
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(count >= 1, "the native worker thread delivered frames");
        assert_eq!(ok(h, "rta", Value::Null)["caps"]["supported"], true);
        ok(h, "rtaStop", Value::Null);
        session_free(h).unwrap();
    }

    #[test]
    fn wp_p0m_meters_carry_the_holds_and_the_event_log_reads() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let m = ok(h, "meters", Value::Null);
        assert_eq!(m["peakHold"].as_array().unwrap().len(), 17);
        assert_eq!(m["peakMax"].as_array().unwrap().len(), 17);
        assert!(m["clipHold"].is_u64());
        let b = ok(
            h,
            "setMeterBallistics",
            json!({ "holdMs": 500, "decayPerS": 1.0, "clipHoldMs": 2000 }),
        );
        assert_eq!(
            b,
            json!({ "holdMs": 500, "decayPerS": 1.0, "clipHoldMs": 2000 })
        );
        assert_eq!(ok(h, "meterBallistics", Value::Null), b);
        assert_eq!(ok(h, "resetMeterMax", Value::Null), Value::Null);
        let page = ok(h, "eventLogRead", json!({}));
        assert_eq!(page["records"][0]["kind"], "connected");
        let last = page["lastId"].as_u64().unwrap();
        assert!(last >= 1);
        let none = ok(h, "eventLogRead", json!({ "afterId": last }));
        assert!(none["records"].as_array().unwrap().is_empty());
        let export = ok(h, "eventLogExport", json!({}));
        assert!(export.as_str().unwrap().contains("\"kind\":\"connected\""));
        assert_eq!(
            ok(h, "eventLogDevices", Value::Null)
                .as_array()
                .unwrap()
                .len(),
            1
        );
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_a_panic_marks_the_module_dead_and_every_later_call_is_refused() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let r = run(h, "test.panic", Value::Null);
        assert_eq!(code(&r), "dead");
        assert!(
            r["error"]
                .as_str()
                .unwrap()
                .contains("test.panic was called")
        );
        let later = run(h, "state", Value::Null);
        assert_eq!(code(&later), "dead");
        assert!(
            later["error"]
                .as_str()
                .unwrap()
                .contains("test.panic was called")
        );
        assert_eq!(session_new().err(), Some(Refusal::Dead));
        guard::reset_for_test();
    }

    // -- WP CG36 --
    #[test]
    fn cg36_crossover_types_by_write_the_delay_line_and_core1_conflict() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let snap = ok(h, "open", json!({ "kind": "demo" }));
        // The first output's wire channel.
        let ch = snap["state"]["numInputChannels"].as_u64().unwrap();
        let ty = |b: u8, t: u8| {
            run(
                h,
                "write",
                json!({ "path": "eq.type", "indices": [ch, b], "value": { "choice": t } }),
            )
        };
        // A crossover type on a crossover band, read back.
        for t in [32u8, 40, 63] {
            let r = ty(20, t);
            assert_eq!(r["value"], json!({ "choice": t }), "{r}");
        }
        assert_eq!(
            ok(h, "state", Value::Null)["state"]["xover"][ch as usize][0]["filterType"],
            63
        );
        // Not on a PEQ band, and no PEQ type on a crossover band.
        let r = ty(3, 40);
        assert_eq!(code(&r), "out_of_range", "{r}");
        assert!(r["error"].as_str().unwrap().contains("PEQ band"), "{r}");
        let r = ty(21, 5);
        assert_eq!(code(&r), "out_of_range", "{r}");
        assert!(r["error"].as_str().unwrap().contains("32..63"), "{r}");
        assert_eq!(ty(3, 13)["value"], json!({ "choice": 13 }));

        // The delay: the platform's line, not the registry's 85 ms.
        let max = snap["state"]["platformId"]
            .as_u64()
            .map(|p| if p == 1 { 42 } else { 21 });
        let max = max.unwrap();
        let r = run(
            h,
            "write",
            json!({ "path": "out.delay", "indices": [0], "value": { "float": max as f32 + 1.0 } }),
        );
        assert_eq!(code(&r), "out_of_range", "{r}");
        assert!(
            r["error"]
                .as_str()
                .unwrap()
                // -- WP CG37 -- the range, 0 to the line.
                .contains(&format!("0 to {max} ms")),
            "{r}"
        );
        let r = run(
            h,
            "write",
            json!({ "path": "out.delay", "indices": [0], "value": { "float": max } }),
        );
        assert_eq!(r["value"], json!({ "float": max as f32 }), "{r}");

        // Ask before an enable: the demo has no conflict.
        assert_eq!(
            ok(h, "core1Conflict", json!({ "output": 0 })),
            json!({ "conflict": false, "reason": null })
        );
        let r = run(h, "core1Conflict", json!({ "output": 200 }));
        assert_eq!(code(&r), "invalid", "{r}");
        session_free(h).unwrap();
    }
    // -- BUG1 --
    #[test]
    fn bug1_the_snapshot_carries_output_labels_and_cards() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let snap = ok(h, "open", json!({ "kind": "demo" }));
        let n = snap["state"]["numOutputChannels"].as_u64().unwrap() as usize;
        assert_eq!(snap["outputLabels"].as_array().unwrap().len(), n);
        assert_eq!(
            snap["outputLabels"][0]["label"], "Out 1 · L · Full",
            "{}",
            snap["outputLabels"]
        );
        // The K340's own configuration on the demo device
        let core = core_of(h).unwrap();
        let doc = include_str!("../../../tests/fixtures/preset/k340-bug1.dspipreset");
        // -- WP P1TZ -- the import with its token.
        let token = core.confirm_token(dspi_core::ConfirmOp::ConfigImport);
        let r: Value = serde_json::from_str(&core.config_import_confirmed(
            doc.into(),
            2,
            u32::MAX,
            Some(token),
        ))
        .unwrap();
        assert_eq!(r["ok"], true, "{r}");
        let snap = ok(h, "state", Value::Null);
        let labels: Vec<&str> = snap["outputLabels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels[..4],
            [
                "Out 1 · R · Low",
                "Out 2 · R · High",
                "Out 3 · L · Low",
                "Out 4 · L · High"
            ]
        );
        assert_eq!(snap["outputLabels"][1]["slotName"], "I2S 1 R");
        assert_eq!(
            snap["outputCards"][0],
            json!({ "left": 2, "right": 0, "title": "Woofers", "leftName": "Woofers L", "rightName": "Woofers R", "leftShortName": "Woofers L", "rightShortName": "R" })
        );
        session_free(h).unwrap();
    }
}
