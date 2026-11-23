//! WP MCP5: the measuring tools, sans-IO, on the demo DSPi with its
//! simulated microphone (P5DEMO). The test plays the app: it takes each
//! request, starts it with `tools::measure::begin` and ticks the core as the
//! app's Measure view does. The plan's flow end to end (preflight, start
//! asked and allowed, status, propose, apply, verify, keep), the low-end
//! search on a tweeter, an unconfirmed layout refused, a role argument
//! refused by the schema, the three settings for test signals, a host
//! without a Measure view, and the requests' lifecycle.

// -- WP ODEC -- the helpers are pub(crate) for tests_wp_odec
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::measure::MeasureRole;
use dspi_core::{
    CoreSet, DeviceId, DeviceTarget, DspiCore, FilterParams, SpeakerLayout, SpeakerLayoutOutput,
};
use serde_json::{Value, json};

use crate::policy::{AgentPolicy, ConfirmMode};
use crate::rate::RateLimit;
use crate::tools::measure::{self, LAYOUT_TEXT, REQUEST_TTL_MS};
use crate::{ClientId, Server, ServerConfig};

const MODERN: &str = "2026-07-28";
const SEED: u64 = 5;

fn request(tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {
    "name": tool, "arguments": args,
    "_meta": {
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "mcp5-agent", "version": "1"},
    }}})
}

pub(crate) fn call_at(s: &mut Server, c: ClientId, now: u64, tool: &str, args: Value) -> Value {
    let r = s.handle_value(c, now, request(tool, args)).unwrap();
    r.get("result").cloned().unwrap_or_else(|| panic!("{r}"))
}

pub(crate) fn ok_at(s: &mut Server, c: ClientId, now: u64, tool: &str, args: Value) -> Value {
    let r = call_at(s, c, now, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    assert_ne!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{tool} was held: {r}"
    );
    r["structuredContent"].clone()
}

pub(crate) fn err_at(s: &mut Server, c: ClientId, now: u64, tool: &str, args: Value) -> String {
    let r = call_at(s, c, now, tool, args);
    assert_eq!(r["isError"], json!(true), "{tool}: {r}");
    r["content"][0]["text"].as_str().unwrap().to_owned()
}

/// A held call's confirmation id.
pub(crate) fn held_at(s: &mut Server, c: ClientId, now: u64, tool: &str, args: Value) -> u64 {
    let r = call_at(s, c, now, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    let d = &r["structuredContent"];
    assert_eq!(d["status"], json!("awaiting_confirmation"), "{tool}: {r}");
    d["confirmation_id"].as_u64().unwrap()
}

/// `tool` held for the user, the user's Allow, and the call's result.
fn allowed(s: &mut Server, c: ClientId, now: u64, tool: &str, args: Value) -> Value {
    let id = held_at(s, c, now, tool, args);
    let risks = s.confirmation(id).unwrap().to_json();
    s.answer_confirmation(id, true, now).unwrap();
    let r = ok_at(
        s,
        c,
        now,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(r["status"], json!("allowed"), "{r}");
    let mut out = r["result"]["structuredContent"].clone();
    out["asked"] = risks["risks"].clone();
    out
}

pub(crate) fn server(set: &Arc<CoreSet>, mode: ConfirmMode, measuring: bool) -> Server {
    let config = ServerConfig {
        policy: Arc::new(AgentPolicy::new(mode)),
        write_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        read_limit: RateLimit {
            burst: 100_000,
            per_s: 1.0,
        },
        measuring,
        ..ServerConfig::default()
    };
    Server::new(Arc::clone(set), config)
}

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The demo DSPi as the simulated room's left three-way (LR4 at 300 Hz and
/// 3 kHz), ready to measure, its simulated microphone on.
pub(crate) fn demo() -> (Arc<CoreSet>, DeviceId, Arc<DspiCore>) {
    let set = CoreSet::new();
    let id = set.open(DeviceTarget::Demo).unwrap();
    let core = set.core(&id).unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let s = core.get_state();
    if s.loudness_enabled {
        core.set_loudness(false).unwrap();
    }
    if s.crossfeed_enabled {
        core.set_crossfeed(false).unwrap();
    }
    if s.psybass_enabled {
        core.set_psybass_enabled(false).unwrap();
    }
    let n_in = s.num_input_channels;
    // P5SIM's three-way chains (`dspi_measure::sim::three_way_chains`).
    const LR4_LP: u8 = 34;
    const LR4_HP: u8 = 35;
    let xo = |filter_type, freq| FilterParams {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    };
    let chains = [
        vec![xo(LR4_LP, 300.0)],
        vec![xo(LR4_HP, 300.0), xo(LR4_LP, 3_000.0)],
        vec![xo(LR4_HP, 3_000.0)],
    ];
    for o in 0..3u8 {
        if !s.output_enabled[usize::from(o)] {
            core.set_output_enable(o, true).unwrap();
        }
        if s.output_muted[usize::from(o)] {
            core.set_output_mute(o, false).unwrap();
        }
        let want = &chains[usize::from(o)];
        let bands = core.get_state().xover[usize::from(n_in + o)].len() as u8;
        for b in 0..bands {
            let p = match want.get(usize::from(b)) {
                Some(x) => *x,
                None => FilterParams {
                    filter_type: 0,
                    bypass: true,
                    freq: 1_000.0,
                    q: 0.707,
                    gain: 0.0,
                    qp: 0.0,
                },
            };
            core.set_crossover(n_in + o, b, p).unwrap();
        }
    }
    settle(&core);
    assert!(core.measure_sim_set(true, SEED).unwrap().on);
    (set, id, core)
}

/// The user's speaker layout: the left three-way, confirmed or not.
pub(crate) fn layout(core: &DspiCore, confirmed: bool) {
    let roles = [MeasureRole::Low, MeasureRole::Mid, MeasureRole::High];
    core.speaker_layout_set(SpeakerLayout {
        outputs: roles
            .iter()
            .enumerate()
            .map(|(o, role)| SpeakerLayoutOutput {
                output: o as u8,
                role: *role,
                speaker: "L".into(),
                group: 0,
            })
            .collect(),
        confirmed_by_user: confirmed,
        ..SpeakerLayout::default()
    })
    .unwrap();
}

/// The app's Measure view: take every waiting request, start it as the app
/// does (the simulated microphone's rate), report it.
pub(crate) fn app_takes(s: &mut Server, core: &DspiCore) -> Vec<Value> {
    let mut started = Vec::new();
    for r in s.measure_requests() {
        let id = r["request_id"].as_u64().unwrap();
        let ticket = s.measure_take(id).unwrap();
        assert!(s.measure_take(id).is_none(), "taken once only");
        match measure::begin(core, &ticket, 48_000) {
            Ok(state) => {
                assert!(s.measure_report(id, true, ""));
                started.push(json!({"ticket": ticket, "state": state}));
            }
            Err(e) => {
                assert!(s.measure_report(id, false, &e));
                started.push(json!({"ticket": ticket, "error": e}));
            }
        }
    }
    started
}

/// The app's loop for a ticket's `loop` until the job ends.
pub(crate) fn app_drives(core: &DspiCore, what: &str, secs: u64) {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let running = match what {
            "session" => core.measure_tick().active,
            "rolloff" => core.measure_rolloff_tick().running,
            "tune" => core.measure_tune_tick().running,
            other => panic!("{other}"),
        };
        if !running {
            return;
        }
        assert!(Instant::now() < end, "{what} did not end in {secs} s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn status(s: &mut Server, c: ClientId, now: u64) -> Value {
    ok_at(s, c, now, "dspi_measure_status", json!({}))
}

// --------------------------------------------------------------- the flow

#[test]
fn wp_mcp5_the_demo_simulation_end_to_end() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::AskRisky, true);
    let c = s.connect();
    let mut now = 1_000;
    let outs = json!({"outputs": [0, 1, 2], "sweep_ms": 1000, "passes": 2});

    // Preflight: the roles read-only from the user's layout.
    let p = ok_at(&mut s, c, now, "dspi_measure_preflight", outs.clone());
    assert_eq!(p["preflight"]["clear"], json!(true), "{p}");
    let roles: Vec<&str> = p["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["low", "mid", "high"]);
    assert_eq!(p["measuring_available"], json!(true));

    // Start: asked (test signals) under the default; Allow queues the request.
    let r = allowed(&mut s, c, now, "dspi_measure_start", outs.clone());
    assert_eq!(r["asked"][0]["kind"], json!("test_signal"), "{r}");
    assert!(
        r["asked"][0]["what"]
            .as_str()
            .unwrap()
            .contains("Out 1, Out 2, Out 3"),
        "{r}"
    );
    assert_eq!(r["status"], json!("waiting_for_app"));
    let req = r["request_id"].as_u64().unwrap();
    assert!(s.take_measure_events(), "the app hears of it");
    assert_eq!(
        status(&mut s, c, now)["request"]["state"],
        json!("waiting_for_app")
    );
    assert!(!core.measure_state().active, "nothing plays before the app");

    // A second start while it waits is busy.
    let e = err_at(&mut s, c, now, "dspi_measure_start", outs.clone());
    assert!(e.contains("waits for the app"), "{e}");

    // The app takes it: the session is the agent's (requested by AI).
    let started = app_takes(&mut s, &core);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["ticket"]["loop"], json!("session"));
    let agent = s.actor_of(c).unwrap();
    assert_eq!(core.measure_state().actor, agent, "{started:?}");
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["state"], json!("running"), "{st}");
    assert_eq!(st["measurement"]["by_this_agent"], json!(true));
    app_drives(&core, "session", 240);
    now += 5_000;
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["request_id"], json!(req));
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    assert_eq!(st["measurement"]["phase"], json!("done"), "{st}");
    assert!(
        st["request"]["note"].as_str().unwrap().contains("complete"),
        "{st}"
    );

    // Propose from this agent's measurement (no store: its document).
    let p = ok_at(&mut s, c, now, "dspi_tune_propose", json!({}));
    let pid = p["proposal_id"].as_u64().unwrap();
    eprintln!("MCP5 proposal: {p}");
    if p["sum_check"] == json!("pending") {
        let r = allowed(
            &mut s,
            c,
            now,
            "dspi_tune_sum_check",
            json!({"proposal_id": pid}),
        );
        assert_eq!(r["asked"][0]["kind"], json!("test_signal"));
        let started = app_takes(&mut s, &core);
        assert_eq!(started[0]["ticket"]["loop"], json!("tune"), "{started:?}");
        app_drives(&core, "tune", 240);
        now += 5_000;
        let st = status(&mut s, c, now);
        assert_eq!(st["request"]["state"], json!("ended"), "{st}");
        assert_eq!(st["tune"]["stage"], json!("proposed"), "{st}");
    }

    // Apply: each write judged as the agent's own (louder ones asked).
    let apply = call_at(
        &mut s,
        c,
        now,
        "dspi_tune_apply",
        json!({"proposal_id": pid, "guard_s": 590}),
    );
    let applied = if apply["structuredContent"]["status"] == json!("awaiting_confirmation") {
        let cid = apply["structuredContent"]["confirmation_id"]
            .as_u64()
            .unwrap();
        s.answer_confirmation(cid, true, now).unwrap();
        let r = ok_at(
            &mut s,
            c,
            now,
            "dspi_get_confirmation",
            json!({"confirmation_id": cid}),
        );
        r["result"].clone()
    } else {
        apply
    };
    if applied["isError"] == json!(true) {
        // A proposal the core will not apply (nothing offered at this
        // level, or its summed check withdrew it): said in words.
        eprintln!("MCP5 apply refused: {applied}");
        assert!(applied["content"][0]["text"].as_str().unwrap().len() > 10);
        return;
    }
    assert_eq!(applied["structuredContent"]["ok"], json!(true), "{applied}");
    assert!(
        core.history(50).iter().any(|e| e.actor == agent),
        "the tune's writes are the agent's in the History"
    );

    // Verify: test signals again, asked; the app runs it.
    let r = allowed(&mut s, c, now, "dspi_tune_verify", json!({}));
    assert_eq!(r["asked"][0]["kind"], json!("test_signal"));
    let started = app_takes(&mut s, &core);
    assert_eq!(
        started[0]["ticket"]["kind"],
        json!("tuneVerify"),
        "{started:?}"
    );
    app_drives(&core, "tune", 300);
    now += 5_000;
    let st = status(&mut s, c, now);
    eprintln!("MCP5 verify: {}", st["tune"]);
    let passed = st["tune"]["verify_passed"].as_bool();
    match passed {
        Some(true) => {
            let k = ok_at(&mut s, c, now, "dspi_tune_keep", json!({}));
            assert_eq!(k["ok"], json!(true), "{k}");
        }
        _ => {
            // A verify that did not pass: the Keep is the user's.
            let e = err_at(&mut s, c, now, "dspi_tune_keep", json!({}));
            assert!(e.contains("only the user"), "{e}");
            ok_at(&mut s, c, now, "dspi_tune_revert", json!({}));
        }
    }
    assert!(core.measure_sim_set(false, 0).is_ok());
}

/// A verify that does not pass leaves the Keep to the user: the user changes
/// the applied tune (a way 12 dB down) before the verify.
#[test]
fn wp_mcp5_a_keep_after_a_failed_verify_is_the_users() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let now = 1_000;
    ok_at(
        &mut s,
        c,
        now,
        "dspi_measure_start",
        json!({"outputs": [0, 1, 2], "sweep_ms": 1000, "passes": 2}),
    );
    app_takes(&mut s, &core);
    app_drives(&core, "session", 240);
    let p = ok_at(
        &mut s,
        c,
        now,
        "dspi_tune_propose",
        json!({"keep_delays": true}),
    );
    let pid = p["proposal_id"].as_u64().unwrap();
    let applied = call_at(
        &mut s,
        c,
        now,
        "dspi_tune_apply",
        json!({"proposal_id": pid, "guard_s": 590}),
    );
    if applied["isError"] == json!(true) {
        eprintln!("MCP5 apply refused: {applied}");
        return;
    }
    // The user turns the mid down by 12 dB: the verify cannot match.
    let g = core.get_state().output_gain_db[1];
    core.set_output_gain(1, g - 12.0).unwrap();
    ok_at(&mut s, c, now, "dspi_tune_verify", json!({}));
    app_takes(&mut s, &core);
    app_drives(&core, "tune", 300);
    let st = status(&mut s, c, now + 1_000);
    assert_eq!(st["tune"]["verify_passed"], json!(false), "{st}");
    let e = err_at(&mut s, c, now, "dspi_tune_keep", json!({}));
    assert!(e.contains("only the user"), "{e}");
    let kept = || {
        matches!(
            core.measure_tune_state().stage,
            dspi_core::measure::MeasureTuneStage::Kept
        ) || core.apply_status().phase == dspi_core::GuardPhase::Kept
    };
    assert!(!kept(), "not kept");

    // -- WP RCFIX -- (review RC, RC1) nor through the generic Keep, however
    // often it asks: two of them, more than the two-step Keep's second apart.
    let keep = |s: &mut Server| {
        let e = err_at(s, c, now, "dspi_keep_changes", json!({}));
        assert!(e.contains("only the user"), "{e}");
        assert!(!kept(), "not kept");
    };
    let confirm = Duration::from_millis(1_100);
    keep(&mut s);
    std::thread::sleep(confirm);
    keep(&mut s);
    // The core itself refuses an agent, whatever tool asks it.
    let agent = dspi_core::actor::register_actor("RCFIX agent");
    for _ in 0..2 {
        let _scope = dspi_core::actor::ActorScope::enter(&core, agent, None);
        assert!(core.apply_confirm().is_err());
        assert!(core.measure_tune_keep().is_err());
        std::thread::sleep(confirm);
    }
    // The user's refused click does not arm the agent's Keep, and the agent's
    // does not spend it: the user's second click keeps.
    let e = core.apply_confirm().unwrap_err().to_string();
    assert!(e.contains("measure.tune.keep_unverified"), "{e}");
    std::thread::sleep(confirm);
    keep(&mut s);
    err_at(&mut s, c, now, "dspi_tune_keep", json!({}));
    let k = core.measure_tune_keep().unwrap();
    assert_eq!(k.guard.phase, dspi_core::GuardPhase::Kept);
}

// ------------------------------------------------------------ the low end

#[test]
fn wp_mcp5_the_low_end_search_on_a_tweeter() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let now = 1_000;
    // Never ask: nothing is asked, the request goes straight to the app.
    let r = ok_at(
        &mut s,
        c,
        now,
        "dspi_measure_rolloff_start",
        json!({"output": 2}),
    );
    assert_eq!(r["status"], json!("waiting_for_app"), "{r}");
    let started = app_takes(&mut s, &core);
    assert_eq!(started[0]["ticket"]["loop"], json!("rolloff"));
    assert_eq!(started[0]["state"]["floorHz"], json!(1000.0), "{started:?}");
    assert_eq!(started[0]["state"]["role"], json!("high"));
    let agent = s.actor_of(c).unwrap();
    assert_eq!(core.measure_rolloff_state().actor, agent);
    // Every generator start on the tweeter, read back while it runs.
    let end = Instant::now() + Duration::from_secs(300);
    let mut lowest = f32::MAX;
    while core.measure_rolloff_tick().running {
        if let Ok(cfg) = core.siggen_get_config()
            && cfg.channel_mask & 0b100 != 0
        {
            // A log sweep's start (`p[0]`): the search comes down from the top.
            lowest = lowest.min(cfg.p.first().copied().unwrap_or(0.0));
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(lowest >= 1_000.0, "the tweeter played from {lowest} Hz");
    let st = status(&mut s, c, now + 1_000);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    let res = &st["rolloff"]["result"];
    let hz = res["rolloff_hz"].as_f64().unwrap_or(0.0);
    assert!((1_000.0..3_000.0).contains(&hz), "{st}");
    assert_eq!(res["stop_db"], json!(15.0));
    assert_eq!(st["rolloff"]["floor_hz"], json!(1000.0));
}

#[test]
fn wp_mcp5_the_low_end_search_of_a_midrange_labelled_tweeter_stops_at_the_floor() {
    let (set, _id, core) = demo();
    layout(&core, true);
    // The user's layout says output 1 (the mid) is a tweeter, and protects
    // it as one (an LR4 high-pass at 1 kHz: the preflight's S6 refuses a
    // tweeter without); the response is still good at 1 kHz.
    let n_in = core.get_state().num_input_channels;
    core.set_crossover(
        n_in + 1,
        0,
        FilterParams {
            filter_type: 35,
            bypass: false,
            freq: 1_000.0,
            q: 0.707,
            gain: 0.0,
            qp: 0.0,
        },
    )
    .unwrap();
    settle(&core);
    let mut l = core.speaker_layout().unwrap().unwrap();
    l.outputs[1].role = MeasureRole::High;
    core.speaker_layout_set(l).unwrap();
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        "dspi_measure_rolloff_start",
        json!({"output": 1}),
    );
    let started = app_takes(&mut s, &core);
    assert_eq!(started[0]["state"]["floorHz"], json!(1000.0), "{started:?}");
    app_drives(&core, "rolloff", 300);
    let st = status(&mut s, c, 1_000);
    let res = &st["rolloff"]["result"];
    assert_eq!(res["end"], json!("floor"), "{st}");
    assert_eq!(
        res["note"]["key"],
        json!("measure.rolloff.midrange"),
        "{st}"
    );
    assert!(res["lowest_hz"].as_f64().unwrap() >= 1_000.0);
}

// ------------------------------------------------------- refusals

#[test]
fn wp_mcp5_an_unconfirmed_layout_is_refused() {
    let (set, id, core) = demo();
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let calls = [
        ("dspi_measure_preflight", json!({})),
        ("dspi_measure_start", json!({"outputs": [0]})),
        ("dspi_measure_rolloff_start", json!({"output": 2})),
        ("dspi_tune_propose", json!({"measurement": "x"})),
        (
            "dspi_system_measure_preflight",
            json!({"units": [{"device": id.as_str()}]}),
        ),
    ];
    // No layout at all, then a layout the user did not confirm (a proposal,
    // an import, another app's).
    for confirmed in [None, Some(false)] {
        if confirmed.is_some() {
            layout(&core, false);
        }
        for (tool, args) in &calls {
            let e = err_at(&mut s, c, 0, tool, args.clone());
            assert!(
                e.contains("Settings › Speakers") && e.contains("not confirmed"),
                "{tool}: {e}"
            );
        }
    }
    assert!(s.measure_requests().is_empty());
    assert!(!core.measure_state().active);
    // Asked for while it was confirmed, not confirmed any more.
    layout(&core, true);
    ok_at(&mut s, c, 0, "dspi_measure_start", json!({"outputs": [0]}));
    let req = s.measure_requests()[0]["request_id"].as_u64().unwrap();
    let ticket = s.measure_take(req).unwrap();
    layout(&core, false);
    assert_eq!(
        measure::begin(&core, &ticket, 48_000).unwrap_err(),
        LAYOUT_TEXT,
        "the core refuses it too, at the app's start"
    );
}

/// Every schema, recursively: no property named like a role.
fn props_named(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Object(p)) = m.get("properties") {
                out.extend(p.keys().cloned());
            }
            m.values().for_each(|x| props_named(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| props_named(x, out)),
        _ => {}
    }
}

#[test]
fn wp_mcp5_a_role_argument_is_rejected_by_the_schema() {
    let (set, id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    for (tool, args) in [
        (
            "dspi_measure_start",
            json!({"outputs": [2], "role": "full"}),
        ),
        ("dspi_measure_preflight", json!({"roles": ["full"]})),
        (
            "dspi_measure_rolloff_start",
            json!({"output": 2, "role": "mid"}),
        ),
        ("dspi_tune_propose", json!({"roles_from_layout": false})),
    ] {
        let e = err_at(&mut s, c, 0, tool, args);
        assert!(e.starts_with("no argument `role"), "{tool}: {e}");
    }
    // Inside a list: an output as an object with a role is not an output.
    let e = err_at(
        &mut s,
        c,
        0,
        "dspi_measure_start",
        json!({"outputs": [{"output": 2, "role": "full"}]}),
    );
    assert!(e.contains("never from an agent"), "{e}");
    let e = err_at(
        &mut s,
        c,
        0,
        "dspi_system_measure_start",
        json!({"units": [{"device": id.as_str(), "outputs": [0], "role": "full"}]}),
    );
    assert!(e.contains("never from an agent"), "{e}");
    assert!(
        s.measure_requests().is_empty(),
        "nothing was asked of the app"
    );
    // No measuring tool's schema has a role anywhere.
    let list = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "mcp5-agent", "version": "1"}}}}),
        )
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in measure::TOOLS {
        assert!(names.contains(&t.name), "{} is listed", t.name);
        let mut props = Vec::new();
        props_named(&t.listed_input_schema(), &mut props);
        assert!(
            !props.iter().any(|p| p.contains("role")),
            "{}: {props:?}",
            t.name
        );
    }
    // There is still no tool that writes the speaker layout.
    assert!(
        !names
            .iter()
            .any(|n| n.contains("layout") || n.contains("speaker"))
    );
}

#[test]
fn wp_mcp5_test_signals_follow_the_setting_and_the_hard_rules_stay() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let args = json!({"outputs": [0]});
    // Ask everything and ask risky: asked. Never ask: not asked.
    for mode in [ConfirmMode::AskEverything, ConfirmMode::AskRisky] {
        let mut s = server(&set, mode, true);
        let c = s.connect();
        let id = held_at(&mut s, c, 0, "dspi_measure_start", args.clone());
        let risks = s.confirmation(id).unwrap().to_json();
        assert_eq!(risks["risks"][0]["kind"], json!("test_signal"), "{risks}");
        s.answer_confirmation(id, false, 0).unwrap();
        assert!(
            s.measure_requests().is_empty(),
            "Deny asks nothing of the app"
        );
    }
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let r = ok_at(&mut s, c, 0, "dspi_measure_start", args.clone());
    assert_eq!(r["status"], json!("waiting_for_app"));
    ok_at(&mut s, c, 0, "dspi_measure_stop", json!({}));

    // The read-only policy refuses (agents not turned on for writing).
    let mut ro = Server::new(
        Arc::clone(&set),
        ServerConfig {
            measuring: true,
            ..ServerConfig::default()
        },
    );
    let c2 = ro.connect();
    let e = err_at(&mut ro, c2, 0, "dspi_measure_start", args.clone());
    assert!(e.contains("does not let agents"), "{e}");

    // Never ask still has the hard rules: the volume over the preflight's
    // cap blocks the start (no question, a refusal).
    core.set_user_volume(0.0).unwrap();
    settle(&core);
    let p = ok_at(&mut s, c, 0, "dspi_measure_preflight", args.clone());
    if p["preflight"]["clear"] == json!(false) {
        let e = err_at(&mut s, c, 0, "dspi_measure_start", args.clone());
        assert!(e.contains("preflight blocks it"), "{e}");
    }
    core.set_user_volume(-20.0).unwrap();

    // The tweeter never gets pink noise or a sweep below 1 kHz: the app's
    // start with a forged setup is refused by the core.
    ok_at(&mut s, c, 0, "dspi_measure_start", args);
    let id = s.measure_requests()[0]["request_id"].as_u64().unwrap();
    let mut forged = s.measure_take(id).unwrap();
    forged["setup"] = json!({"kind": "pink", "outputs": [{"output": 2}], "rolesFromLayout": true});
    let e = measure::begin(&core, &forged, 48_000).unwrap_err();
    assert!(e.contains("measure.tweeter_stimulus"), "{e}");
    s.measure_report(id, false, &e);
    assert!(!core.measure_state().active);
}

#[test]
fn wp_mcp5_without_a_measure_view_measuring_is_not_available() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, false);
    let c = s.connect();
    let p = ok_at(
        &mut s,
        c,
        0,
        "dspi_measure_preflight",
        json!({"outputs": [0]}),
    );
    assert_eq!(p["measuring_available"], json!(false));
    for (tool, args) in [
        ("dspi_measure_start", json!({"outputs": [0]})),
        ("dspi_measure_rolloff_start", json!({"output": 2})),
        ("dspi_tune_verify", json!({})),
    ] {
        let e = err_at(&mut s, c, 0, tool, args);
        assert!(e.contains("Measure view"), "{tool}: {e}");
    }
    assert!(s.measure_requests().is_empty());
}

// A way of the user's layout measured alone, with no input routed to it yet:
// the device's own suggestion leaves such an output out.
#[test]
fn wp_func1_a_proposal_takes_the_layouts_outputs_routed_or_not() {
    let (set, _id, core) = demo();
    layout(&core, true);
    for i in 0..core.get_state().num_input_channels {
        core.set_matrix_route(i, 0, false, 0.0, false).unwrap();
    }
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let args = json!({"outputs": [0], "sweep_ms": 1000, "passes": 2});
    ok_at(&mut s, c, 1_000, "dspi_measure_start", args);
    app_takes(&mut s, &core);
    app_drives(&core, "session", 240);
    status(&mut s, c, 6_000);
    ok_at(&mut s, c, 6_000, "dspi_tune_propose", json!({}));
}

// The first real agent stopped here: the overview said it may not measure.
#[test]
fn wp_func1_the_overview_says_an_agent_may_measure_where_it_may() {
    let (set, _id, _core) = demo();
    for measuring in [true, false] {
        let mut s = server(&set, ConfirmMode::AskRisky, measuring);
        let c = s.connect();
        let o = ok_at(&mut s, c, 0, "dspi_get_overview", json!({}));
        assert_eq!(o["agent_may"]["measure"], json!(measuring));
        let l = ok_at(&mut s, c, 0, "dspi_get_limits", json!({}));
        assert_eq!(l["agent_may"]["measure"], json!(measuring));
    }
}

#[test]
fn wp_mcp5_requests_lapse_are_declined_stopped_and_withdrawn() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let args = json!({"outputs": [0], "sweep_ms": 1000});
    let mut now = 1_000;

    // Nobody takes it: it lapses, nothing played.
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    now += REQUEST_TTL_MS + 1;
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["state"], json!("lapsed"), "{st}");
    assert!(s.measure_requests().is_empty());

    // The app cannot start it (no microphone): declined with the reason.
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    let id = s.measure_requests()[0]["request_id"].as_u64().unwrap();
    s.measure_take(id).unwrap();
    assert!(s.measure_report(id, false, "no microphone is chosen"));
    assert!(!s.measure_report(id, true, ""), "reported once");
    let st = status(&mut s, c, now);
    assert_eq!(st["request"]["state"], json!("declined"));
    assert_eq!(st["request"]["note"], json!("no microphone is chosen"));

    // The agent's Stop withdraws a waiting one, and stops a running one at
    // once (the generator silenced).
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    let r = ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    assert_eq!(r["stopped"], json!(["request"]));
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    app_takes(&mut s, &core);
    for _ in 0..20 {
        core.measure_tick();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(core.measure_state().active);
    let r = ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    assert_eq!(r["stopped"], json!(["measurement"]), "{r}");
    assert!(!core.measure_state().active);

    // Another agent cannot stop it; nor move it on.
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    app_takes(&mut s, &core);
    let other = s.connect();
    let r = ok_at(&mut s, other, now, "dspi_measure_stop", json!({}));
    assert_eq!(r["stopped"], json!([]), "{r}");
    assert!(core.measure_state().active);
    // Agents turned off: what they started stops, what waits is withdrawn.
    s.stop_agents(now);
    assert!(!core.measure_state().active);

    // -- WP RCFIX -- (review RC, RC4) A taken one does not lapse while the app
    // opens the microphone; it starts and the agent reads that it runs.
    let take = |s: &mut Server| {
        let id = s.measure_requests()[0]["request_id"].as_u64().unwrap();
        (id, s.measure_take(id).unwrap())
    };
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    let (id, ticket) = take(&mut s);
    now += REQUEST_TTL_MS + 1;
    s.tick(now);
    assert_eq!(
        status(&mut s, c, now)["request"]["state"],
        json!("starting")
    );
    measure::begin(&core, &ticket, 48_000).unwrap();
    assert!(s.measure_report(id, true, ""));
    assert_eq!(status(&mut s, c, now)["request"]["state"], json!("running"));
    // Its ticket starts nothing again.
    core.measure_stop();
    assert!(measure::begin(&core, &ticket, 48_000).is_err());
    s.tick(now + 1_000);
    // The agent's Stop, or the agent gone, while the app opens the
    // microphone: nothing starts, and the app hears it may not run.
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    let (id, ticket) = take(&mut s);
    ok_at(&mut s, c, now, "dspi_measure_stop", json!({}));
    let e = measure::begin(&core, &ticket, 48_000).unwrap_err();
    assert!(e.contains("withdrawn"), "{e}");
    assert!(!s.measure_report(id, true, ""));
    let gone = s.connect();
    ok_at(&mut s, gone, now, "dspi_measure_start", args.clone());
    let (id, ticket) = take(&mut s);
    s.disconnect(gone);
    let e = measure::begin(&core, &ticket, 48_000).unwrap_err();
    assert!(e.contains("withdrawn"), "{e}");
    assert!(!core.measure_state().active, "nothing was played");
    assert!(!s.measure_report(id, true, ""));
    let log = s.measure_log();
    assert_eq!(log.last().unwrap()["state"], json!("cancelled"), "{log:?}");
    // One taken from a server that is gone.
    {
        let mut other = server(&set, ConfirmMode::NeverAsk, true);
        let c2 = other.connect();
        ok_at(&mut other, c2, now, "dspi_measure_start", args.clone());
        let (_, ticket) = take(&mut other);
        drop(other);
        assert!(measure::begin(&core, &ticket, 48_000).is_err());
        assert!(!core.measure_state().active);
    }

    // A client gone: its waiting request is withdrawn.
    ok_at(&mut s, c, now, "dspi_measure_start", args.clone());
    s.disconnect(c);
    assert!(s.measure_requests().is_empty());
    let log = s.measure_log();
    assert_eq!(log.last().unwrap()["state"], json!("cancelled"), "{log:?}");
}

#[test]
fn wp_mcp5_the_measurement_list_and_read() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("wp_mcp5_list_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    core.measure_set_dir(dir.to_string_lossy().into_owned())
        .unwrap();
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        "dspi_measure_start",
        json!({"outputs": [0], "sweep_ms": 1000, "passes": 2}),
    );
    app_takes(&mut s, &core);
    app_drives(&core, "session", 120);
    let st = status(&mut s, c, 1_000);
    let filed = st["request"]["filed_measurement_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!filed.is_empty(), "{st}");
    let list = ok_at(&mut s, c, 1_000, "dspi_list_measurements", json!({}));
    assert!(
        list["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == json!(filed)),
        "{list}"
    );
    // MCP1's read gives its curve.
    let m = ok_at(
        &mut s,
        c,
        1_000,
        "dspi_get_response",
        json!({"output": 0, "source": "measurement", "measurement_id": filed}),
    );
    assert!(!m["freqs_hz"].as_array().unwrap().is_empty(), "{m}");
    // A tune proposed from the filed id.
    let p = ok_at(
        &mut s,
        c,
        1_000,
        "dspi_tune_propose",
        json!({"measurement": filed}),
    );
    assert!(p["proposal_id"].is_u64(), "{p}");
    let _ = std::fs::remove_dir_all(&dir);
}

// -- WP RCFIX -- (review RC, RC5)
/// The user's low-end search, started before the tick that sees the agent's
/// end, is never filed under the agent's request.
#[test]
fn wp_rcfix_the_users_search_is_not_the_agents() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let args = json!({"output": 2, "start_hz": 4000});
    let r = ok_at(&mut s, c, 0, "dspi_measure_rolloff_start", args);
    assert_eq!(r["status"], json!("waiting_for_app"), "{r}");
    let id = r["request_id"].as_u64().unwrap();
    let what = s.measure_requests()[0]["what"].clone();
    assert!(what.as_str().unwrap().contains("from 4 kHz"), "{what}");
    app_takes(&mut s, &core);
    assert_eq!(status(&mut s, c, 100)["request"]["state"], json!("running"));
    core.measure_rolloff_stop();
    let mine = core
        .measure_rolloff_start(dspi_core::measure::MeasureRolloffSetup {
            output: 2,
            sweep_ms: 1_000,
            start_level_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(mine.actor, dspi_core::actor::EMBEDDER);
    let st = status(&mut s, c, 1_000);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    let note = st["request"]["note"].as_str().unwrap();
    assert!(note.contains("replaced"), "{note}");
    assert_eq!(st["request"]["filed_measurement_id"], json!(""), "{st}");
    // The user's runs on, and ends as the user's.
    assert!(core.measure_rolloff_state().running);
    core.measure_rolloff_stop();
    s.tick(2_000);
    let log = s.measure_log();
    let r = log
        .iter()
        .find(|r| r["request_id"].as_u64() == Some(id))
        .unwrap();
    assert_eq!(r["note"].as_str(), Some(note), "{log:?}");
}

// -- WP FFIX -- FUNC1 bug 5: the long poll's mark
#[test]
fn wp_ffix_the_status_mark_changes_when_the_app_takes_the_request() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let poll = request("dspi_measure_status", json!({"wait_s": 25}));
    assert_eq!(Server::long_poll_ms(&poll), Some(25_000));
    assert_eq!(
        Server::long_poll_ms(&request("dspi_measure_status", json!({"wait_s": 99}))),
        Some(25_000),
        "at most 25 s"
    );
    assert_eq!(
        Server::long_poll_ms(&request("dspi_measure_status", json!({}))),
        None
    );
    assert_eq!(
        Server::long_poll_ms(&request("dspi_get_overview", json!({"wait_s": 5}))),
        None
    );
    // Idle jobs are left out of the answer.
    let st = status(&mut s, c, 1_000);
    assert!(
        st.get("tune").is_none() && st.get("rolloff").is_none(),
        "{st}"
    );
    let outs = json!({"outputs": [0, 1, 2], "sweep_ms": 1000, "passes": 2});
    ok_at(&mut s, c, 1_000, "dspi_measure_start", outs);
    let m0 = s.status_mark(c, 1_000, &poll).unwrap();
    assert_eq!(
        s.status_mark(c, 1_100, &poll).unwrap(),
        m0,
        "nothing changed"
    );
    app_takes(&mut s, &core);
    let m1 = s.status_mark(c, 1_200, &poll).unwrap();
    assert_ne!(m1, m0, "the request runs now");
    core.measure_stop();
    app_drives(&core, "session", 60);
}

// -- WP FFIX -- FUNC1 bug 6: one microphone for every device
#[test]
fn wp_ffix_a_second_measurement_on_another_device_is_busy() {
    let (set, id1, core) = demo();
    layout(&core, true);
    let id2 = set.open(DeviceTarget::Demo).unwrap();
    let core2 = set.core(&id2).unwrap();
    core2.fetch_all().unwrap();
    layout(&core2, true);
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let outs = json!({"outputs": [0], "sweep_ms": 1000, "passes": 2, "device": id1.as_str()});
    ok_at(&mut s, c, 1_000, "dspi_measure_start", outs.clone());
    let mut on2 = outs.clone();
    on2["device"] = json!(id2.as_str());
    // While the first waits for the app, and while it runs.
    let e = err_at(&mut s, c, 1_000, "dspi_measure_start", on2.clone());
    assert!(
        e.contains("waits for the app on demo:1") && e.contains("one microphone"),
        "{e}"
    );
    app_takes(&mut s, &core);
    let e = err_at(&mut s, c, 1_000, "dspi_measure_start", on2.clone());
    assert!(e.contains("a measurement is running on demo:1"), "{e}");
    let mut search = json!({"output": 0, "device": id2.as_str()});
    search["start_hz"] = json!(4000);
    let e = err_at(&mut s, c, 1_000, "dspi_measure_rolloff_start", search);
    assert!(e.contains("one microphone"), "{e}");
    core.measure_stop();
    app_drives(&core, "session", 60);
}

// -- WP FFIX -- FUNC1 bugs 3, 9 and 15: arrivals, the alignment across
// speakers from one run, and what a filed measurement holds
#[test]
fn wp_ffix_one_run_gives_arrivals_and_the_alignment_across_speakers() {
    let (set, _id, core) = demo();
    core.speaker_layout_set(SpeakerLayout {
        outputs: [(0u8, "L", 0u8), (1, "R", 1)]
            .iter()
            .map(|(o, sp, g)| SpeakerLayoutOutput {
                output: *o,
                role: MeasureRole::Low,
                speaker: (*sp).into(),
                group: *g,
            })
            .collect(),
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    })
    .unwrap();
    let mut s = server(&set, ConfirmMode::NeverAsk, true);
    let c = s.connect();
    let args = json!({"outputs": [0, 1], "sweep_ms": 2000, "passes": 3, "one_run": true});
    let p = ok_at(&mut s, c, 1_000, "dspi_measure_preflight", args.clone());
    assert_eq!(p["preflight"]["clear"], json!(true), "{p}");
    ok_at(&mut s, c, 1_000, "dspi_measure_start", args);
    app_takes(&mut s, &core);
    assert_eq!(core.measure_state().runs, 1, "both speakers in one run");
    app_drives(&core, "session", 240);
    let st = status(&mut s, c, 6_000);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    let outs = st["result"]["outputs"].as_array().unwrap();
    assert_eq!(outs.len(), 2, "{st}");
    assert!(outs.iter().all(|o| o["arrival_ms"].is_number()), "{st}");
    // Out 2 has the demo's 300 Hz LR4 high-pass: the result names it.
    assert!(
        outs[1]["highpass"][0]
            .as_str()
            .unwrap()
            .contains("at 300 Hz"),
        "{st}"
    );
    let a = &st["alignment"];
    assert_eq!(a["speakers"].as_array().unwrap().len(), 2, "{st}");
    assert!(a["changes"].is_array() && a["note"].as_str().unwrap().contains("lowest way"));
    // The response read says what it includes, and when it arrived.
    let id = st["request"]["filed_measurement_id"]
        .as_str()
        .map(str::to_owned);
    let doc_id = id.unwrap_or_default();
    if !doc_id.is_empty() {
        let r = ok_at(
            &mut s,
            c,
            6_000,
            "dspi_get_response",
            json!({"source": "measurement", "measurement_id": doc_id, "output": 1}),
        );
        assert!(r["arrival_ms"].is_number(), "{r}");
        assert!(
            r["includes"]
                .as_str()
                .unwrap()
                .contains("not the driver's own"),
            "{r}"
        );
        let l = ok_at(&mut s, c, 6_000, "dspi_list_measurements", json!({}));
        assert_eq!(
            l["measurements"][0]["holds"]["kind"],
            json!("walked"),
            "{l}"
        );
    }
    // A plain walked run is one run per speaker.
    ok_at(
        &mut s,
        c,
        7_000,
        "dspi_measure_start",
        json!({"outputs": [0, 1], "sweep_ms": 1000, "passes": 2}),
    );
    app_takes(&mut s, &core);
    assert_eq!(core.measure_state().runs, 2);
    core.measure_stop();
    app_drives(&core, "session", 60);
}
