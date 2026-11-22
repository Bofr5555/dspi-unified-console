//! WP MCP2a: the policy, confirmations and the edit tools, on demo devices,
//! sans-IO: louder held and answered, the three settings, the tweeter rule
//! (asked, and refused by the user's setup), the batch with its guard, undo of
//! one's own changes only, the drag retry with its staleness bound, the
//! user's open undo group, hard refusals never asked, and the rate limits.

use std::sync::Arc;

use dspi_core::actor::EMBEDDER;
use dspi_core::measure::MeasureRole;
use dspi_core::{
    CoreSet, DeviceId, DeviceTarget, DspiCore, FilterParams, SpeakerLayout, SpeakerLayoutOutput,
};
use serde_json::{Map, Value, json};

use crate::confirm::{AnswerError, CONFIRM_HOLD_MS, ConfirmEvent, ConfirmState};
use crate::policy::{self, AgentPolicy, ConfirmMode, RiskKind};
use crate::rate::RateLimit;
use crate::registry::{Call, Registry, Tool, ToolClass, ToolResult};
use crate::server::{Handled, STALE_DRAG_MS};
use crate::{ClientId, Server, ServerConfig};

const MODERN: &str = "2026-07-28";

fn set_of(n: usize) -> (Arc<CoreSet>, Vec<DeviceId>) {
    let set = CoreSet::new();
    let ids = (0..n)
        .map(|_| set.open(DeviceTarget::Demo).unwrap())
        .collect();
    (set, ids)
}

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test-agent", "version": "1"},
    })
}

fn request(tool: &str, args: Value) -> Value {
    let mut params = json!({"name": tool, "arguments": args});
    params["_meta"] = meta();
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params})
}

fn call_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> Value {
    let r = s.handle_value(c, now_ms, request(tool, args)).unwrap();
    r.get("result").cloned().unwrap_or_else(|| panic!("{r}"))
}

fn ok_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> Value {
    let r = call_at(s, c, now_ms, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    assert_ne!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{tool} was held: {r}"
    );
    r["structuredContent"].clone()
}

fn ok(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    ok_at(s, c, 0, tool, args)
}

fn err_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> String {
    let r = call_at(s, c, now_ms, tool, args);
    assert_eq!(r["isError"], json!(true), "{tool}: {r}");
    r["content"][0]["text"].as_str().unwrap().to_owned()
}

fn err(s: &mut Server, c: ClientId, tool: &str, args: Value) -> String {
    err_at(s, c, 0, tool, args)
}

/// A held call's confirmation id (the non-holding transport's answer).
fn held_at(s: &mut Server, c: ClientId, now_ms: u64, tool: &str, args: Value) -> u64 {
    let r = call_at(s, c, now_ms, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    let d = &r["structuredContent"];
    assert_eq!(d["status"], json!("awaiting_confirmation"), "{tool}: {r}");
    d["confirmation_id"].as_u64().unwrap()
}

fn held(s: &mut Server, c: ClientId, tool: &str, args: Value) -> u64 {
    held_at(s, c, 0, tool, args)
}

fn agent_server(set: &Arc<CoreSet>, mode: ConfirmMode) -> (Server, Arc<AgentPolicy>) {
    let policy = Arc::new(AgentPolicy::new(mode));
    let config = ServerConfig {
        policy: policy.clone(),
        // Many writes at one instant of the test clock.
        write_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        ..ServerConfig::default()
    };
    (Server::new(Arc::clone(set), config), policy)
}

fn n_in(core: &DspiCore) -> u8 {
    core.get_state().num_input_channels
}

fn lr4_highpass(hz: f32) -> FilterParams {
    FilterParams {
        filter_type: 35,
        bypass: false,
        freq: hz,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    }
}

fn gain(core: &DspiCore, o: usize) -> f32 {
    core.get_state().output_gain_db[o]
}

fn xover(core: &DspiCore, o: u8, x: usize) -> FilterParams {
    core.get_state().xover[usize::from(n_in(core) + o)][x]
}

/// The user's confirmed speaker setup: output `o` is `role`, the others full.
fn confirm_layout(core: &DspiCore, o: u8, role: MeasureRole) {
    let outputs = (0..core.get_state().num_output_channels)
        .map(|n| SpeakerLayoutOutput {
            output: n,
            role: if n == o { role } else { MeasureRole::Full },
            speaker: "L".into(),
            group: 0,
        })
        .collect();
    core.speaker_layout_set(SpeakerLayout {
        outputs,
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    })
    .unwrap();
}

// ------------------------------------------------- louder: held, answered

#[test]
fn wp_mcp2a_a_louder_change_is_held_and_allow_applies_it_deny_refuses_it() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    // Quieter passes at once, as one journal entry of this agent's.
    let r = ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -6.0}),
    );
    assert_eq!(gain(&core, 0), -6.0);
    assert_eq!(r["changes"][0]["before"], json!(0.0));
    assert_eq!(r["changes"][0]["after"], json!(-6.0));
    let undo_id = r["undo_id"].as_u64().unwrap();
    let h = core.history(1);
    assert_eq!(h[0].id, undo_id);
    assert_eq!(h[0].actor, s.actor_of(c).unwrap());

    // Louder is held: nothing sent, the app hears of it, the agent is told.
    let _ = s.take_confirm_events();
    let id = held(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -1.0}),
    );
    assert_eq!(gain(&core, 0), -6.0);
    assert_eq!(s.take_confirm_events(), vec![ConfirmEvent::Asked(id)]);
    let sheet = s.waiting_confirmations();
    assert_eq!(sheet.len(), 1);
    assert_eq!(sheet[0]["tool"], json!("dspi_set_gain"));
    assert_eq!(sheet[0]["risks"][0]["kind"], json!("louder"));
    assert_eq!(sheet[0]["risks"][0]["output"], json!(0));
    assert!(
        sheet[0]["risks"][0]["what"]
            .as_str()
            .unwrap()
            .contains("Out 1 gain up from -6.0 dB to -1.0 dB"),
        "{sheet:?}"
    );
    assert_eq!(s.activity(1)[0].outcome, crate::activity::Outcome::Held);
    let w = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(w["status"], json!("waiting"));

    // Allow applies exactly that, as the agent's.
    s.answer_confirmation(id, true, 1_000).unwrap();
    assert_eq!(gain(&core, 0), -1.0);
    assert_eq!(core.history(1)[0].actor, s.actor_of(c).unwrap());
    assert_eq!(
        s.take_confirm_events(),
        vec![ConfirmEvent::Ended(id, ConfirmState::Allowed)]
    );
    let w = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(w["status"], json!("allowed"));
    assert_eq!(w["result"]["isError"], json!(false));
    assert_eq!(
        w["result"]["structuredContent"]["changes"][0]["after"],
        json!(-1.0)
    );
    // Answered once only.
    assert_eq!(
        s.answer_confirmation(id, false, 1_000),
        Err(AnswerError::Ended(ConfirmState::Allowed))
    );
    assert_eq!(
        s.answer_confirmation(999, true, 0),
        Err(AnswerError::Unknown)
    );

    // Deny: nothing changed, and the agent is told not to retry.
    let id = held(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 3.0}),
    );
    s.answer_confirmation(id, false, 2_000).unwrap();
    assert_eq!(gain(&core, 0), -1.0);
    let w = ok(
        &mut s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert_eq!(w["status"], json!("denied"));
    assert!(
        w["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("declined")
    );
    // Another agent cannot read this agent's confirmation.
    let c2 = s.connect();
    let e = err(
        &mut s,
        c2,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    );
    assert!(e.contains("no confirmation"), "{e}");
}

#[test]
fn wp_mcp2a_a_held_request_is_answered_when_the_user_answers_or_after_120_s() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    core.set_output_mute(1, true).unwrap();

    // A transport that holds: the unmute waits, then the reply comes.
    let text = request("dspi_set_mute", json!({"output": 1, "muted": false})).to_string();
    let Handled::Held(id) = s.handle_held(c, 0, &text) else {
        panic!("not held")
    };
    assert_eq!(s.resume(id, 10), None);
    s.answer_confirmation(id, true, 20).unwrap();
    let reply: Value = serde_json::from_str(&s.resume(id, 30).unwrap()).unwrap();
    assert_eq!(reply["id"], json!(7));
    assert_eq!(reply["result"]["isError"], json!(false));
    assert_eq!(reply["result"]["resultType"], json!("complete"));
    assert!(!core.get_state().output_muted[1]);
    // Answered once.
    assert_eq!(s.resume(id, 40), None);

    // Nobody answers: after 120 s the agent hears so, nothing changed.
    let text = request("dspi_set_gain", json!({"output": 1, "gain_db": 6.0})).to_string();
    let Handled::Held(id) = s.handle_held(c, 1_000, &text) else {
        panic!("not held")
    };
    assert_eq!(s.resume(id, 1_000 + CONFIRM_HOLD_MS - 1), None);
    let reply: Value =
        serde_json::from_str(&s.resume(id, 1_000 + CONFIRM_HOLD_MS).unwrap()).unwrap();
    assert_eq!(reply["result"]["isError"], json!(true));
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("did not answer")
    );
    assert_eq!(gain(&core, 1), 0.0);
    // A lapsed one can no longer be allowed.
    assert_eq!(
        s.answer_confirmation(id, true, 1_000 + CONFIRM_HOLD_MS + 1),
        Err(AnswerError::Unknown)
    );

    // A hold the non-holding path opened is not taken for a later request's.
    let _ = held_at(
        &mut s,
        c,
        150_000,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": 9.0}),
    );
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list",
                      "params": {"_meta": meta()}})
    .to_string();
    assert!(matches!(
        s.handle_held(c, 150_001, &list),
        Handled::Reply(Some(_))
    ));
    // A call that needs nobody is answered at once.
    let text = request("dspi_set_gain", json!({"output": 1, "gain_db": -3.0})).to_string();
    assert!(matches!(
        s.handle_held(c, 200_000, &text),
        Handled::Reply(Some(_))
    ));

    // The agent gone: its waiting confirmation ends, nothing changed.
    let id = held_at(
        &mut s,
        c,
        300_000,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": 2.0}),
    );
    s.disconnect(c);
    assert_eq!(s.confirmation(id).unwrap().state, ConfirmState::Gone);
    assert_eq!(gain(&core, 1), -3.0);
}

// ---------------------------------------------------------- the settings

#[test]
fn wp_mcp2a_the_three_settings_ask_everything_risky_or_nothing() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, policy) = agent_server(&set, ConfirmMode::AskEverything);
    let c = s.connect();
    let flags = ok(&mut s, c, "dspi_get_overview", json!({}))["agent_may"].clone();
    assert_eq!(flags["confirm"], json!("ask_everything"));
    assert_eq!(flags["write"], json!(true));

    // Ask on everything: even a quieter change.
    let id = held(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -3.0}),
    );
    assert_eq!(s.confirmation(id).unwrap().risks[0].kind, RiskKind::Change);
    assert_eq!(gain(&core, 0), 0.0);
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(gain(&core, 0), -3.0);

    // Never ask: louder, unmute, boosts and test signals all pass.
    policy.set_mode(ConfirmMode::NeverAsk);
    ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 6.0}),
    );
    assert_eq!(gain(&core, 0), 6.0);
    ok(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"output": 0, "slot": "free", "type": "peaking", "freq_hz": 100, "gain_db": 12}),
    );
    assert!(s.waiting_confirmations().is_empty());

    // Ask for risky changes: louder asks, quieter passes.
    policy.set_mode(ConfirmMode::AskRisky);
    ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 2.0}),
    );
    held(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 4.0}),
    );
    assert_eq!(ConfirmMode::parse("never_ask"), Some(ConfirmMode::NeverAsk));
    assert_eq!(ConfirmMode::default(), ConfirmMode::AskRisky);
}

/// A tool that starts test signals, the way MCP5's will ask: a static class.
static TEST_SIGNAL: Tool = Tool {
    name: "test_measure_start",
    title: "t",
    description: "t",
    class: ToolClass::Write,
    device: true,
    input_schema: || json!({"type": "object", "properties": {}}),
    output_schema: || json!({"type": "object"}),
    handler: test_signal,
};

fn test_signal(call: &mut Call<'_>, _: &Map<String, Value>) -> ToolResult {
    policy::require(
        &call.judging,
        RiskKind::TestSignal,
        "Play test signals on Out 1",
        &[0],
    )?;
    Ok(json!({"started": true}))
}

#[test]
fn wp_mcp2a_test_signals_are_asked_by_default_and_never_under_never_ask() {
    let (set, _) = set_of(1);
    let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
    let config = ServerConfig {
        policy: policy.clone(),
        ..ServerConfig::default()
    };
    let mut s = Server::with_registry(Arc::clone(&set), config, Registry::of(vec![&TEST_SIGNAL]));
    let c = s.connect();
    let id = held(&mut s, c, "test_measure_start", json!({}));
    assert_eq!(
        s.confirmation(id).unwrap().risks[0].kind,
        RiskKind::TestSignal
    );
    s.answer_confirmation(id, true, 0).unwrap();
    let w = s.confirmation(id).unwrap();
    assert_eq!(w.state, ConfirmState::Allowed);
    policy.set_mode(ConfirmMode::NeverAsk);
    assert_eq!(
        ok(&mut s, c, "test_measure_start", json!({}))["started"],
        json!(true)
    );
    // Under a policy that is not the agent policy it is refused.
    let mut s = Server::with_registry(
        Arc::clone(&set),
        ServerConfig::default(),
        Registry::of(vec![&TEST_SIGNAL]),
    );
    let c = s.connect();
    let e = err(&mut s, c, "test_measure_start", json!({}));
    assert!(e.contains("does not let agents"), "{e}");
}

// ------------------------------------------------------- the tweeter rule

#[test]
fn wp_mcp2a_an_hp_removed_on_an_output_with_hp_above_1khz_is_asked_then_applied() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let ch = n_in(&core) + 1;
    core.set_crossover(ch, 0, lr4_highpass(2_000.0)).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    // Moving it up, or to another slot at the same corner, passes: the plan is
    // judged whole.
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 2500}}),
    );
    assert_eq!(xover(&core, 1, 0).freq, 2_500.0);
    // A bandpass: the low-pass joins the free slot.
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "low_pass": {"family": "butterworth", "order": 2, "freq_hz": 12000}}),
    );
    assert_eq!(xover(&core, 1, 1).filter_type, 42);

    // Removing it, moving it below 1 kHz or making it 6 dB/oct is asked.
    for hp in [
        Value::Null,
        json!({"family": "linkwitz_riley", "order": 4, "freq_hz": 800}),
        json!({"family": "butterworth", "order": 1, "freq_hz": 2500}),
    ] {
        let id = held(
            &mut s,
            c,
            "dspi_set_crossover",
            json!({"output": 1, "high_pass": hp}),
        );
        let w = s.confirmation(id).unwrap();
        assert_eq!(w.risks[0].kind, RiskKind::TweeterCrossing);
        assert_eq!(w.risks[0].output, Some(1));
        assert_eq!(xover(&core, 1, 0).freq, 2_500.0);
        s.answer_confirmation(id, false, 0).unwrap();
    }
    // So is a PEQ high-pass slot that the output's protection rests on, and a
    // bypass of the crossover band.
    let id = held(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_crossover", "output": 1, "high_pass": null}]}),
    );
    s.answer_confirmation(id, false, 0).unwrap();

    // Allowed, it goes: exactly what the user saw.
    let id = held(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": null}),
    );
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(xover(&core, 1, 0).filter_type, 0);
    assert_eq!(xover(&core, 1, 1).filter_type, 42);
}

#[test]
fn wp_mcp2a_never_ask_refuses_a_tweeter_crossing_and_the_users_tweeter_is_refused_always() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let ch = n_in(&core) + 1;
    core.set_crossover(ch, 0, lr4_highpass(2_000.0)).unwrap();
    let (mut s, policy) = agent_server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();

    // Never ask never asks: the crossing is refused (owner 2026-11-22; this
    // supersedes the plan's "never ask still asks for the tweeter crossing").
    let e = err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": null}),
    );
    assert!(e.contains("never ask"), "{e}");
    assert!(s.waiting_confirmations().is_empty());
    assert_eq!(xover(&core, 1, 0).freq, 2_000.0);

    // The user's setup says output 1 is a tweeter: refused in every mode,
    // never asked, even with its high-pass below 1 kHz to start from.
    confirm_layout(&core, 1, MeasureRole::High);
    for mode in [
        ConfirmMode::AskEverything,
        ConfirmMode::AskRisky,
        ConfirmMode::NeverAsk,
    ] {
        policy.set_mode(mode);
        let e = err(
            &mut s,
            c,
            "dspi_set_crossover",
            json!({"output": 1, "high_pass": null}),
        );
        assert!(
            e.contains("tweeter in the user's speaker setup"),
            "{mode:?}: {e}"
        );
        assert!(s.waiting_confirmations().is_empty(), "{mode:?}");
        // A batch is refused whole.
        let e = err(
            &mut s,
            c,
            "dspi_apply_changes",
            json!({"changes": [{"op": "clear_peq_band", "output": 1, "slot": 0},
                               {"op": "set_crossover", "output": 1,
                                "high_pass": {"family": "bessel", "order": 2, "freq_hz": 500}}]}),
        );
        assert!(e.contains("speaker setup"), "{e}");
    }
    assert_eq!(xover(&core, 1, 0).freq, 2_000.0);
    // Raising it is fine.
    policy.set_mode(ConfirmMode::AskRisky);
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 3000}}),
    );
    // A midrange in the setup keeps 80 Hz.
    confirm_layout(&core, 2, MeasureRole::Mid);
    core.set_crossover(ch + 1, 0, lr4_highpass(300.0)).unwrap();
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 100}}),
    );
    let e = err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": null}),
    );
    assert!(e.contains("a midrange"), "{e}");
    // An unconfirmed setup is not the user's word: asked, not refused.
    let mut layout = core.speaker_layout().unwrap().unwrap();
    layout.confirmed_by_user = false;
    core.speaker_layout_set(layout).unwrap();
    held(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": null}),
    );
}

#[test]
fn wp_mcp2a_boosts_delay_steps_and_routes_are_judged() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let band = json!({"input": 0, "slot": 3, "type": "peaking", "freq_hz": 200, "q": 2});
    let with = |g: f64| {
        let mut b = band.clone();
        b["gain_db"] = json!(g);
        b
    };
    ok(&mut s, c, "dspi_set_peq_band", with(5.5));
    ok(&mut s, c, "dspi_set_peq_band", with(-12.0));
    let id = held(&mut s, c, "dspi_set_peq_band", with(8.0));
    assert_eq!(s.confirmation(id).unwrap().risks[0].kind, RiskKind::Boost);
    assert_eq!(core.get_state().filters[0][3].gain, -12.0);
    s.answer_confirmation(id, false, 0).unwrap();
    // "free" takes the first flat slot.
    let r = ok(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"input": 0, "slot": "free", "type": "notch", "freq_hz": 50}),
    );
    assert_eq!(r["changes"][0]["slot"], json!(0));

    // Delay: small steps pass, a jump is asked.
    ok(
        &mut s,
        c,
        "dspi_set_delay",
        json!({"output": 0, "delay_ms": 5.0}),
    );
    let id = held(
        &mut s,
        c,
        "dspi_set_delay",
        json!({"output": 0, "delay_ms": 40.0}),
    );
    assert_eq!(
        s.confirmation(id).unwrap().risks[0].kind,
        RiskKind::DelayStep
    );
    s.answer_confirmation(id, false, 0).unwrap();

    // Routing: a new route is asked, cutting one passes.
    let st = core.get_state();
    let (i, o) = (0u8, 1u8);
    let was_on = st.matrix_routing[0][1];
    if was_on {
        ok(
            &mut s,
            c,
            "dspi_set_route",
            json!({"input": i, "output": o, "on": false}),
        );
    }
    let id = held(
        &mut s,
        c,
        "dspi_set_route",
        json!({"input": i, "output": o, "on": true}),
    );
    assert_eq!(
        s.confirmation(id).unwrap().risks[0].kind,
        RiskKind::NewRoute
    );
    s.answer_confirmation(id, false, 0).unwrap();
    // Volume: down passes, up asks; an enable asks.
    ok(
        &mut s,
        c,
        "dspi_set_volume",
        json!({"volume": "user", "db": -30}),
    );
    held(
        &mut s,
        c,
        "dspi_set_volume",
        json!({"volume": "user", "db": -10}),
    );
    ok(
        &mut s,
        c,
        "dspi_set_output_enabled",
        json!({"output": 0, "enabled": false}),
    );
    let id = held(
        &mut s,
        c,
        "dspi_set_output_enabled",
        json!({"output": 0, "enabled": true}),
    );
    assert_eq!(s.confirmation(id).unwrap().risks[0].kind, RiskKind::Enable);
    s.answer_confirmation(id, false, 0).unwrap();
    // The fourth waiting one of one agent is not opened.
    held(
        &mut s,
        c,
        "dspi_set_volume",
        json!({"volume": "user", "db": -5}),
    );
    held(
        &mut s,
        c,
        "dspi_set_volume",
        json!({"volume": "user", "db": -4}),
    );
    let e = err(
        &mut s,
        c,
        "dspi_set_volume",
        json!({"volume": "user", "db": -3}),
    );
    assert!(e.contains("not yet answered"), "{e}");
}

// ------------------------------------------------------------- the batch

#[test]
fn wp_mcp2a_a_batch_is_one_undo_step_all_or_none_and_may_be_guarded() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, policy) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    let r = ok(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [
            {"op": "set_gain", "output": 0, "gain_db": -4},
            {"op": "set_gain", "output": 1, "gain_db": -5},
            {"op": "set_peq_band", "output": 0, "slot": "free", "type": "peaking",
             "freq_hz": 3000, "gain_db": -3}]}),
    );
    assert_eq!(r["changes"].as_array().unwrap().len(), 3);
    let ids_of_call = r["undo_ids"].as_array().unwrap().clone();
    assert_eq!(ids_of_call.len(), 1, "{r}");
    let h = core.history(3);
    assert!(h.iter().all(|e| e.id == h[0].id));
    assert_eq!(h[0].label.as_deref(), Some("AI: several changes"));

    // One risky change holds the whole batch: nothing is sent.
    let id = held(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -8},
                           {"op": "set_gain", "output": 1, "gain_db": 3}]}),
    );
    assert_eq!(gain(&core, 0), -4.0);
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!((gain(&core, 0), gain(&core, 1)), (-8.0, 3.0));

    // A device refusal halfway puts back what came before (never ask, so the
    // long delay is the device's to refuse).
    policy.set_mode(ConfirmMode::NeverAsk);
    let e = err(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -9},
                           {"op": "set_delay", "output": 0, "delay_ms": 999}]}),
    );
    assert!(e.contains("put back"), "{e}");
    assert_eq!(gain(&core, 0), -8.0);
    policy.set_mode(ConfirmMode::AskRisky);
    // Bad changes are named by position, and a change cannot pick a device.
    let e = err(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -1},
                           {"op": "flash_firmware"}]}),
    );
    assert!(e.starts_with("change 1:"), "{e}");
    let e = err(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -1, "device": "x"}]}),
    );
    assert!(e.contains("one device"), "{e}");

    // Guarded: a timed preview, reverted on request or kept.
    let r = ok(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -20}], "guard_s": 30}),
    );
    let guard = r["guard"]["guard_id"].as_u64().unwrap();
    assert_eq!(r["guard"]["phase"], json!("waiting"));
    assert_eq!(gain(&core, 0), -20.0);
    s.tick(1_000);
    assert_eq!(core.apply_status().phase, dspi_core::GuardPhase::Waiting);
    // Another agent cannot end it.
    let c2 = s.connect();
    let e = err(&mut s, c2, "dspi_revert_changes", json!({}));
    assert!(e.contains("no timed preview"), "{e}");
    let r = ok(&mut s, c, "dspi_revert_changes", json!({"guard_id": guard}));
    assert_eq!(r["guard"]["phase"], json!("reverted"));
    assert_eq!(gain(&core, 0), -8.0);
    ok(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -12}], "guard_s": 30}),
    );
    let r = ok(&mut s, c, "dspi_keep_changes", json!({}));
    assert_eq!(r["guard"]["phase"], json!("kept"));
    assert_eq!(gain(&core, 0), -12.0);
    let e = err(&mut s, c, "dspi_keep_changes", json!({}));
    assert!(e.contains("no timed preview"), "{e}");
}

// ------------------------------------------------------------------- undo

#[test]
fn wp_mcp2a_undo_touches_only_its_own_entries() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let a = s.connect();
    let b = s.connect();

    // The user's change, then agent a's, then agent b's.
    core.set_output_gain(2, -2.0).unwrap();
    let user_id = core.history(1)[0].id;
    assert_eq!(core.history(1)[0].actor, EMBEDDER);
    let ra = ok(
        &mut s,
        a,
        "dspi_set_delay",
        json!({"output": 0, "delay_ms": 3.0}),
    );
    let rb = ok(
        &mut s,
        b,
        "dspi_set_delay",
        json!({"output": 1, "delay_ms": 4.0}),
    );

    // Neither may undo the user's, nor the other agent's.
    let e = err(&mut s, a, "dspi_undo", json!({"undo_id": user_id}));
    assert!(e.contains("not this agent's"), "{e}");
    let e = err(&mut s, a, "dspi_undo", json!({"undo_id": rb["undo_id"]}));
    assert!(e.contains("not this agent's"), "{e}");
    assert_eq!(core.get_state().output_delay_ms[1], 4.0);

    // By default an agent undoes its own newest change.
    let r = ok(&mut s, a, "dspi_undo", json!({}));
    assert_eq!(r["undone"], ra["undo_id"]);
    assert_eq!(core.get_state().output_delay_ms[0], 0.0);
    assert_eq!(core.get_state().output_delay_ms[1], 4.0);
    assert_eq!(gain(&core, 2), -2.0);
    let e = err(&mut s, a, "dspi_undo", json!({}));
    assert!(
        e.contains("already undone") || e.contains("no change"),
        "{e}"
    );

    // Undoing a cut is louder: judged like any change.
    ok(
        &mut s,
        b,
        "dspi_set_gain",
        json!({"output": 3, "gain_db": -10}),
    );
    let id = held(&mut s, b, "dspi_undo", json!({}));
    assert_eq!(s.confirmation(id).unwrap().risks[0].kind, RiskKind::Louder);
    assert_eq!(gain(&core, 3), -10.0);
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(gain(&core, 3), 0.0);
}

// ----------------------------------------- the user editing, groups, hard

#[test]
fn wp_mcp2a_a_drag_refuses_agents_until_released_or_stale() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    core.set_dragging(true);
    let e = err_at(
        &mut s,
        c,
        0,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -1}),
    );
    assert!(e.contains("editing"), "{e}");
    // The user keeps moving the control: never stale.
    core.set_output_gain(5, -1.0).unwrap();
    err_at(
        &mut s,
        c,
        6_000,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -1}),
    );
    core.set_output_gain(5, -1.5).unwrap();
    err_at(
        &mut s,
        c,
        12_000,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -1}),
    );
    assert_eq!(gain(&core, 0), 0.0);
    assert!(
        s.waiting_confirmations().is_empty(),
        "editing is never asked"
    );
    // Released: the agent's write goes.
    core.set_dragging(false);
    ok_at(
        &mut s,
        c,
        13_000,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -1}),
    );

    // A drag nobody releases (a control destroyed while pressed): after
    // STALE_DRAG_MS with no change by the user, the server clears the mark
    // and the write goes.
    core.set_dragging(true);
    err_at(
        &mut s,
        c,
        20_000,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -2}),
    );
    err_at(
        &mut s,
        c,
        20_000 + STALE_DRAG_MS - 1,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -2}),
    );
    ok_at(
        &mut s,
        c,
        20_000 + STALE_DRAG_MS,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -2}),
    );
    assert_eq!(gain(&core, 0), -2.0);
}

#[test]
fn wp_mcp2a_a_batch_waits_while_the_users_undo_group_is_open() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    core.begin_group("the user's".into());
    assert_eq!(core.undo_group_owner(), Some(EMBEDDER));
    let batch = json!({"changes": [{"op": "set_gain", "output": 0, "gain_db": -1},
                                   {"op": "set_gain", "output": 1, "gain_db": -1}]});
    let e = err(&mut s, c, "dspi_apply_changes", batch.clone());
    assert!(e.contains("middle of a change"), "{e}");
    assert_eq!(gain(&core, 0), 0.0);
    core.end_group();
    assert_eq!(core.undo_group_owner(), None);
    ok(&mut s, c, "dspi_apply_changes", batch);
    assert_eq!(core.undo_group_owner(), None);
}

/// A tool that tries what only the user may: set the speaker setup.
static SET_ROLES: Tool = Tool {
    name: "test_set_roles",
    title: "t",
    description: "t",
    class: ToolClass::Write,
    device: true,
    input_schema: || json!({"type": "object", "properties": {}}),
    output_schema: || json!({"type": "object"}),
    handler: |call, _| {
        let core = call.core()?;
        core.speaker_layout_set(SpeakerLayout::default())
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({}))
    },
};

#[test]
fn wp_mcp2a_user_only_and_read_only_are_hard_refusals_never_asked() {
    let (set, _) = set_of(1);
    let (mut s, _) = {
        let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskEverything));
        let config = ServerConfig {
            policy: policy.clone(),
            ..ServerConfig::default()
        };
        (
            Server::with_registry(Arc::clone(&set), config, Registry::of(vec![&SET_ROLES])),
            policy,
        )
    };
    let c = s.connect();
    let e = err(&mut s, c, "test_set_roles", json!({}));
    assert!(e.contains("only the user"), "{e}");
    assert!(s.waiting_confirmations().is_empty());
    assert!(policy::hard_code("user_only"));
    assert!(policy::hard_code("agent_read_only"));
    assert!(policy::hard_code("not_own_entry"));
    assert!(!policy::hard_code("user_editing"));

    // The default policy is read-only: the edit tools are refused, not asked.
    let mut s = Server::new(Arc::clone(&set), ServerConfig::default());
    let c = s.connect();
    let e = err(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 3}),
    );
    assert!(e.contains("only read"), "{e}");
    assert!(s.waiting_confirmations().is_empty());
}

// -------------------------------------------------------------- the rest

#[test]
fn wp_mcp2a_writes_are_rate_limited_on_the_test_clock_and_allows_are_not() {
    let (set, _) = set_of(1);
    let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
    let config = ServerConfig {
        policy,
        write_limit: RateLimit {
            burst: 2,
            per_s: 1.0,
        },
        ..ServerConfig::default()
    };
    let mut s = Server::new(Arc::clone(&set), config);
    let c = s.connect();
    let quiet = json!({"output": 0, "gain_db": -1});
    ok_at(&mut s, c, 0, "dspi_set_gain", quiet.clone());
    let id = held_at(
        &mut s,
        c,
        0,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": 1}),
    );
    let e = err_at(&mut s, c, 0, "dspi_set_gain", quiet.clone());
    assert!(e.contains("wait 1000 ms"), "{e}");
    assert_eq!(
        s.activity(1)[0].outcome,
        crate::activity::Outcome::RateLimited
    );
    // The user's Allow is not the agent's call: never rate-limited.
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(s.confirmation(id).unwrap().state, ConfirmState::Allowed);
    ok_at(&mut s, c, 1_000, "dspi_set_gain", quiet);
}

#[test]
fn wp_mcp2a_the_tool_list_has_the_edit_tools_and_none_of_the_agent_proof_ones() {
    let (set, _) = set_of(1);
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta()}}),
        )
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for w in [
        "dspi_set_peq_band",
        "dspi_clear_peq_band",
        "dspi_set_peq_bypass",
        "dspi_set_eq_bypass",
        "dspi_set_crossover",
        "dspi_set_delay",
        "dspi_set_gain",
        "dspi_set_polarity",
        "dspi_set_mute",
        "dspi_set_output_enabled",
        "dspi_set_route",
        "dspi_set_volume",
        "dspi_set_user_mute",
        "dspi_undo",
        "dspi_apply_changes",
        "dspi_keep_changes",
        "dspi_revert_changes",
    ] {
        let t = tools.iter().find(|t| t["name"] == json!(w)).unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], json!(false), "{w}");
        assert!(
            t["inputSchema"]["properties"].get("device").is_some(),
            "{w}"
        );
    }
    assert!(names.contains(&"dspi_get_confirmation"));
    for proof in [
        "flash",
        "bootloader",
        "picoboot",
        "factory",
        "vreg",
        "pin",
        "unlock",
        "limits_set",
        "raw",
        "siggen",
        "identify",
        "pair",
        "speaker_layout",
        "delete",
        "close_device",
    ] {
        assert!(
            !names.iter().any(|n| n.contains(proof)),
            "{proof} in {names:?}"
        );
    }
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "a name twice");
}

#[test]
fn wp_mcp2a_two_devices_each_judged_on_its_own() {
    let (set, ids) = set_of(2);
    let (a, b) = (set.core(&ids[0]).unwrap(), set.core(&ids[1]).unwrap());
    b.set_crossover(n_in(&b) + 1, 0, lr4_highpass(2_000.0))
        .unwrap();
    let (mut s, _) = agent_server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    // No high-pass on a: removing nothing passes. On b it is asked.
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"device": ids[0].as_str(), "output": 1, "high_pass": null}),
    );
    let id = held(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"device": ids[1].as_str(), "output": 1, "high_pass": null}),
    );
    assert_eq!(
        s.confirmation(id).unwrap().device.as_deref(),
        Some(ids[1].as_str())
    );
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(xover(&b, 1, 0).filter_type, 0);
    let _ = a;
}

/// Tools that write with no plan of their own, so only the core's hook
/// judges them (as a later tool's wholesale write would be).
static RAW_LOUDER: Tool = Tool {
    name: "test_raw_louder",
    title: "t",
    description: "t",
    class: ToolClass::Write,
    device: true,
    input_schema: || json!({"type": "object", "properties": {}}),
    output_schema: || json!({"type": "object"}),
    handler: |call, _| {
        let core = call.core()?;
        core.set_output_gain(0, 5.0)
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({}))
    },
};

static RAW_NO_HP: Tool = Tool {
    name: "test_raw_no_hp",
    title: "t",
    description: "t",
    class: ToolClass::Write,
    device: true,
    input_schema: || json!({"type": "object", "properties": {}}),
    output_schema: || json!({"type": "object"}),
    handler: |call, _| {
        let core = call.core()?;
        let ch = n_in(core) + 1;
        core.set_crossover(
            ch,
            0,
            FilterParams {
                filter_type: 0,
                ..lr4_highpass(2_000.0)
            },
        )
        .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({}))
    },
};

#[test]
fn wp_mcp2a_the_hook_judges_writes_no_plan_named() {
    let (set, ids) = set_of(1);
    let core = set.core(&ids[0]).unwrap();
    core.set_crossover(n_in(&core) + 1, 0, lr4_highpass(2_000.0))
        .unwrap();
    let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
    let config = ServerConfig {
        policy: policy.clone(),
        ..ServerConfig::default()
    };
    let mut s = Server::with_registry(
        Arc::clone(&set),
        config,
        Registry::of(vec![&RAW_LOUDER, &RAW_NO_HP]),
    );
    let c = s.connect();
    // The hook holds the louder write before it is sent; Allow passes it.
    let id = held(&mut s, c, "test_raw_louder", json!({}));
    assert_eq!(gain(&core, 0), 0.0);
    assert_eq!(s.confirmation(id).unwrap().risks[0].kind, RiskKind::Louder);
    s.answer_confirmation(id, true, 0).unwrap();
    assert_eq!(gain(&core, 0), 5.0);
    // The crossing through the hook: asked, then refused under never ask, and
    // refused for the user's tweeter.
    let id = held(&mut s, c, "test_raw_no_hp", json!({}));
    s.answer_confirmation(id, false, 0).unwrap();
    policy.set_mode(ConfirmMode::NeverAsk);
    let e = err(&mut s, c, "test_raw_no_hp", json!({}));
    assert!(e.contains("never ask"), "{e}");
    policy.set_mode(ConfirmMode::AskRisky);
    confirm_layout(&core, 1, MeasureRole::High);
    let e = err(&mut s, c, "test_raw_no_hp", json!({}));
    assert!(e.contains("speaker setup"), "{e}");
    assert_eq!(xover(&core, 1, 0).filter_type, 35);
    assert!(s.waiting_confirmations().is_empty());
}
