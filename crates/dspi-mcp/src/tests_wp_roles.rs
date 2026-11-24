//! WP ROLES: an agent sets the speaker roles from the user's words (owner
//! 2026-11-23). The one question with the whole table, "never ask" only with
//! the user's switch, a confirmed setup only with the words on what changed,
//! the overview's and the preflight's `speaker_setup_stated`, the tweeter
//! rules on stated and unnamed outputs, and trust but check: a stated
//! non-tweeter's top-fall check before anything plays below 1 kHz, end to
//! end on the demo DSPi with its simulated microphone.

use std::sync::Arc;

use dspi_core::measure::MeasureRole;
use dspi_core::{CoreSet, LayoutState, RoleCheckVerdict, RoleSource};
use serde_json::{Value, json};

use crate::policy::{AgentPolicy, ConfirmMode, ROLES_NEVER_ASK_TEXT};
use crate::rate::RateLimit;
use crate::tests_wp_mcp5::{
    app_drives, app_takes, call_at, demo, err_at, held_at, layout, ok_at, status,
};
use crate::tools::roles::{ROLES_CONFIRMED_TEXT, SET_ROLES_NAME};
use crate::{Server, ServerConfig};

const ROLES: &str = SET_ROLES_NAME;

/// The owner's sentence, verbatim.
const OWNER: &str = "L and r front speakers, both on DSPi named main front, full range three \
                     way, tweeter,mid,woofer, l then right, output 1-6";

fn server(set: &Arc<CoreSet>, mode: ConfirmMode) -> (Server, Arc<AgentPolicy>) {
    let policy = Arc::new(AgentPolicy::new(mode));
    let config = ServerConfig {
        policy: Arc::clone(&policy) as Arc<dyn crate::Policy>,
        write_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        read_limit: RateLimit {
            burst: 100_000,
            per_s: 1.0,
        },
        measuring: true,
        ..ServerConfig::default()
    };
    (Server::new(Arc::clone(set), config), policy)
}

/// The owner's table: Out 1 L tweeter, 2 L mid, 3 L woofer, 4 R tweeter, 5
/// R mid, 6 R woofer (outputs from 0).
fn owners_table() -> Value {
    json!({"user_words": OWNER, "rows": [
        {"output": 0, "role": "tweeter", "speaker": "L"},
        {"output": 1, "role": "midrange", "speaker": "L"},
        {"output": 2, "role": "woofer", "speaker": "L"},
        {"output": 3, "role": "tweeter", "speaker": "R"},
        {"output": 4, "role": "midrange", "speaker": "R"},
        {"output": 5, "role": "woofer", "speaker": "R"},
    ]})
}

fn overview(s: &mut Server, c: crate::ClientId) -> Value {
    ok_at(s, c, 0, "dspi_get_overview", json!({}))
}

/// Allow confirmation `cid` and read the held call's result.
fn allow(s: &mut Server, c: crate::ClientId, cid: u64) -> Value {
    s.answer_confirmation(cid, true, 0).unwrap();
    let r = ok_at(
        s,
        c,
        0,
        "dspi_get_confirmation",
        json!({"confirmation_id": cid}),
    );
    assert_eq!(r["status"], json!("allowed"), "{r}");
    r["result"]["structuredContent"].clone()
}

#[test]
fn wp_roles_the_owners_sentence_is_one_question_with_the_whole_table() {
    for mode in [ConfirmMode::AskRisky, ConfirmMode::AskEverything] {
        let (set, _id, core) = demo();
        let (mut s, _) = server(&set, mode);
        let c = s.connect();
        let n = core.get_state().num_output_channels;
        assert_eq!(overview(&mut s, c)["agent_may"]["set_roles"], json!(true));
        let cid = held_at(&mut s, c, 0, ROLES, owners_table());
        // Nothing kept while it waits.
        assert!(core.speaker_layout().unwrap().is_none());
        let sheet = s.confirmation(cid).unwrap().to_json();
        let risks = sheet["risks"].as_array().unwrap();
        assert_eq!(risks.len(), 1, "one question: {sheet}");
        assert_eq!(risks[0]["kind"], json!("speaker_roles"));
        let what = risks[0]["what"].as_str().unwrap();
        assert!(
            what.starts_with(
                "Speaker roles from your words: Out 1 · L · tweeter; Out 2 · L · midrange; \
                 Out 3 · L · woofer; Out 4 · R · tweeter; Out 5 · R · midrange; Out 6 · R · woofer."
            ),
            "{what}"
        );
        assert!(
            what.contains("not named: unknown, treated as tweeters"),
            "{what}"
        );
        assert!(
            what.contains("full range three way"),
            "the words quoted: {what}"
        );
        let table = &sheet["roles"];
        assert_eq!(table["rows"].as_array().unwrap().len(), 6, "{sheet}");
        assert_eq!(
            table["rows"][4],
            json!({"output": 4, "speaker": "R", "role": "midrange", "role_text": "midrange"})
        );
        assert_eq!(table["user_words"], json!(OWNER));
        assert!(sheet["grant"].is_null(), "no test signals: {sheet}");
        let r = allow(&mut s, c, cid);
        assert_eq!(r["roles_from"], json!("speaker_setup_stated"), "{r}");
        assert_eq!(
            r["unknown_treated_as_tweeter"],
            json!((6..n).collect::<Vec<u8>>())
        );
        let l = core.speaker_layout().unwrap().unwrap();
        assert_eq!(l.state(), LayoutState::Stated);
        let roles: Vec<(u8, MeasureRole, &str, u8)> = l
            .outputs
            .iter()
            .map(|o| (o.output, o.role, o.speaker.as_str(), o.group))
            .collect();
        use MeasureRole::{High, Low, Mid};
        assert_eq!(
            roles,
            [
                (0, High, "L", 0),
                (1, Mid, "L", 0),
                (2, Low, "L", 0),
                (3, High, "R", 1),
                (4, Mid, "R", 1),
                (5, Low, "R", 1)
            ]
        );
        assert!(l.outputs.iter().all(|o| o.source == RoleSource::Stated));
        let st = l.stated.unwrap();
        assert_eq!(st.words, OWNER);
        assert!(st.by.starts_with("AI · "), "{}", st.by);
        // The overview says so, per output too.
        let o = overview(&mut s, c);
        assert_eq!(o["roles_from"], json!("speaker_setup_stated"), "{o}");
        assert_eq!(o["outputs"][1]["role"], json!("mid"));
        assert_eq!(o["outputs"][1]["role_source"], json!("stated"));
        assert_eq!(
            o["outputs"][1]["treated_as_tweeter"],
            json!(false),
            "a midrange as stated, checked or not (ROLESFIX)"
        );
        assert_eq!(
            o["outputs"][0]["treated_as_tweeter"],
            json!(false),
            "a tweeter as stated"
        );
        assert_eq!(o["outputs"][6]["treated_as_tweeter"], json!(true), "{o}");
        // The preflight too.
        let p = ok_at(
            &mut s,
            c,
            0,
            "dspi_measure_preflight",
            json!({"outputs": [1]}),
        );
        assert_eq!(p["roles_from"], json!("speaker_setup_stated"), "{p}");
        assert_eq!(p["roles"][0]["source"], json!("stated"));
        assert_eq!(p["roles"][0]["below_1khz_waits_for_check"], json!(true));
        // Stating a different table is a new question; denied, nothing moves.
        let mut other = owners_table();
        other["rows"][1]["role"] = json!("woofer");
        let cid = held_at(&mut s, c, 0, ROLES, other);
        s.answer_confirmation(cid, false, 0).unwrap();
        let l = core.speaker_layout().unwrap().unwrap();
        assert_eq!(l.output(1).unwrap().role, Mid);
    }
}

#[test]
fn wp_roles_never_ask_only_with_the_users_switch() {
    let (set, _id, core) = demo();
    let (mut s, policy) = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    assert_eq!(overview(&mut s, c)["agent_may"]["set_roles"], json!(false));
    let e = err_at(&mut s, c, 0, ROLES, owners_table());
    assert_eq!(e, ROLES_NEVER_ASK_TEXT);
    assert!(core.speaker_layout().unwrap().is_none());
    policy.set_agent_roles(true);
    assert_eq!(overview(&mut s, c)["agent_may"]["set_roles"], json!(true));
    let r = ok_at(&mut s, c, 0, ROLES, owners_table());
    assert_eq!(r["roles_from"], json!("speaker_setup_stated"));
    assert_eq!(
        core.speaker_layout().unwrap().unwrap().state(),
        LayoutState::Stated
    );
    // The detection's switch is another one.
    assert!(!policy.agent_detect());
}

#[test]
fn wp_roles_a_confirmed_setup_only_with_the_words_on_what_changed() {
    let (set, _id, core) = demo();
    layout(&core, true);
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let words = json!({"user_words": "output 2 is now the tweeter",
                       "rows": [{"output": 1, "role": "tweeter", "speaker": "L"}]});
    let e = err_at(&mut s, c, 0, ROLES, words.clone());
    assert_eq!(e, ROLES_CONFIRMED_TEXT);
    assert_eq!(
        core.speaker_layout().unwrap().unwrap().state(),
        LayoutState::Confirmed
    );
    let mut changed = words;
    changed["what_changed"] = json!("I rewired: output 2 is now the tweeter");
    let cid = held_at(&mut s, c, 0, ROLES, changed);
    let sheet = s.confirmation(cid).unwrap().to_json();
    assert_eq!(sheet["risks"].as_array().unwrap().len(), 1);
    let what = sheet["risks"][0]["what"].as_str().unwrap();
    assert!(
        what.contains("This replaces your confirmed setup"),
        "{what}"
    );
    assert!(what.contains("I rewired"), "{what}");
    assert_eq!(
        sheet["roles"]["what_changed"],
        json!("I rewired: output 2 is now the tweeter")
    );
    allow(&mut s, c, cid);
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(l.state(), LayoutState::Stated);
    assert_eq!(l.outputs.len(), 1);
    assert_eq!(
        l.stated.unwrap().what_changed,
        "I rewired: output 2 is now the tweeter"
    );
}

#[test]
fn wp_roles_the_tweeter_rules_hold_on_stated_and_unnamed_outputs() {
    for mode in [ConfirmMode::AskRisky, ConfirmMode::NeverAsk] {
        let (set, _id, core) = demo();
        let (mut s, policy) = server(&set, mode);
        policy.set_agent_roles(true);
        let c = s.connect();
        let rows = json!({"user_words": "1 woofer, 2 mid, 3 tweeter", "rows": [
            {"output": 0, "role": "woofer", "speaker": "L"},
            {"output": 1, "role": "midrange", "speaker": "L"},
            {"output": 2, "role": "tweeter", "speaker": "L"}]});
        match mode {
            ConfirmMode::NeverAsk => {
                ok_at(&mut s, c, 0, ROLES, rows);
            }
            _ => {
                let cid = held_at(&mut s, c, 0, ROLES, rows);
                allow(&mut s, c, cid);
            }
        }
        // The stated tweeter keeps its high-pass, worded.
        let e = err_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": null}),
        );
        assert!(
            e.contains("drives a tweeter")
                && e.contains("set by AI from the user's words, not yet confirmed"),
            "{e}"
        );
        // -- ROLESFIX -- (owner 2026-11-24) the boundary: music may reach a
        // stated woofer before any check (routing, filters, unmute follow the
        // stated role); a test signal below 1 kHz there waits for the check
        for i in 0..core.get_state().num_input_channels {
            core.set_matrix_route(i, 0, false, 0.0, false).unwrap();
        }
        let woofer = json!({"output": 0, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 40}});
        ok_at(&mut s, c, 0, "dspi_set_crossover", woofer);
        let route = json!({"input": 0, "output": 0, "on": true});
        match mode {
            ConfirmMode::NeverAsk => {
                ok_at(&mut s, c, 0, "dspi_set_route", route);
            }
            _ => {
                // a new route is asked, as anywhere
                let cid = held_at(&mut s, c, 0, "dspi_set_route", route);
                allow(&mut s, c, cid);
            }
        }
        let st = core.get_state();
        assert!(
            (0..st.num_input_channels).any(|i| st.matrix_routing[usize::from(i)][0]),
            "routed before any check"
        );
        assert!(
            core.speaker_layout()
                .unwrap()
                .unwrap()
                .check_of(0)
                .is_none(),
            "unchecked"
        );
        if mode == ConfirmMode::NeverAsk {
            let r = ok_at(&mut s, c, 0, "dspi_measure_start", json!({"outputs": [0]}));
            assert!(
                r["what"]
                    .as_str()
                    .unwrap()
                    .starts_with("First a check that Out 1 (stated woofer) is not a tweeter"),
                "{r}"
            );
            ok_at(&mut s, c, 0, "dspi_measure_stop", json!({}));
        }
        // (the core holds an agent's plan for it to 1 kHz until then:
        // dspi-core's wp_roles_an_agent_states_the_roles_and_its_stimulus_waits_for_the_check)
        // An output the words left out is guarded as a tweeter: while it plays
        // (the user switched it on and routed it), no high-pass under 1 kHz.
        core.set_output_enable(3, true).unwrap();
        core.set_output_mute(3, false).unwrap();
        core.set_matrix_route(0, 3, true, 0.0, false).unwrap();
        let e = err_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 3, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 500}}),
        );
        assert!(
            e.contains("is not among the roles the user stated")
                && e.contains("guarded as a tweeter"),
            "{e}"
        );
    }
}

#[test]
fn wp_roles_trust_but_check_a_wrong_woofer_is_caught_before_anything_plays_low() {
    let (set, _id, core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    // The demo's Out 3 is a tweeter; the user calls it a woofer.
    let cid = held_at(
        &mut s,
        c,
        0,
        ROLES,
        json!({"user_words": "1 woofer, 2 mid, 3 woofer", "rows": [
            {"output": 0, "role": "woofer", "speaker": "L"},
            {"output": 1, "role": "midrange", "speaker": "L"},
            {"output": 2, "role": "woofer", "speaker": "L"}]}),
    );
    allow(&mut s, c, cid);
    // Its low end: the test signals are asked, then the check goes first.
    let cid = held_at(
        &mut s,
        c,
        0,
        "dspi_measure_rolloff_start",
        json!({"output": 2}),
    );
    let r = allow(&mut s, c, cid);
    assert!(
        r["what"]
            .as_str()
            .unwrap()
            .starts_with("First a check that Out 3 (stated woofer) is not a tweeter"),
        "{r}"
    );
    let first = r["request_id"].as_u64().unwrap();
    let started = app_takes(&mut s, &core);
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["ticket"]["loop"], json!("detect"), "{started:?}");
    assert!(started[0].get("error").is_none(), "{started:?}");
    assert!(core.detect_state().check);
    let e = err_at(&mut s, c, 0, "dspi_measure_start", json!({}));
    assert!(
        e.contains("a check of the stated speaker roles is running"),
        "{e}"
    );
    app_drives(&core, "detect", 300);
    let st = status(&mut s, c, 5_000);
    // The check's request ended, the search went on as its own.
    let checked = ok_at(
        &mut s,
        c,
        5_000,
        "dspi_measure_status",
        json!({"request_id": first}),
    );
    let next = st["request"]["request_id"].as_u64().unwrap();
    assert_ne!(next, first, "{st}");
    assert_eq!(
        checked["request"]["request_id"],
        json!(next),
        "followed: {checked}"
    );
    assert_eq!(
        st["detect"]["checks"][0]["verdict"],
        json!("looks_like_tweeter"),
        "{st}"
    );
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(
        l.check_of(2).map(|c| c.verdict),
        Some(RoleCheckVerdict::TopHolds),
        "{l:#?}"
    );
    let o = overview(&mut s, c);
    assert_eq!(
        o["outputs"][2]["role_check"],
        json!("looks_like_tweeter"),
        "{o}"
    );
    assert_eq!(o["outputs"][2]["treated_as_tweeter"], json!(true));
    // With that evidence the whole tweeter rule holds, playing or not.
    let e = err_at(
        &mut s,
        c,
        5_000,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": {"family": "linkwitz_riley", "order": 4, "freq_hz": 600}}),
    );
    assert!(
        e.contains(
            "was stated a woofer in the user's words, but its top end holds like a tweeter's"
        ),
        "{e}"
    );
    // The search plays from 1 kHz, no lower.
    let started = app_takes(&mut s, &core);
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(
        started[0]["ticket"]["loop"],
        json!("rolloff"),
        "{started:?}"
    );
    assert!(started[0].get("error").is_none(), "{started:?}");
    assert!(core.measure_rolloff_state().floor_hz >= 1_000.0);
    app_drives(&core, "rolloff", 300);
    let st = status(&mut s, c, 10_000);
    assert_eq!(st["request"]["state"], json!("ended"), "{st}");
    let note = st["request"]["note"].as_str().unwrap();
    assert!(
        note.starts_with("Out 3 looks like a tweeter: the user's words made it a woofer"),
        "{note}"
    );
    assert!(note.contains("Then: the low-end search"), "{note}");
    // Not checked again: the next search goes straight on, still from 1 kHz.
    let r = ok_at(
        &mut s,
        c,
        10_000,
        "dspi_measure_rolloff_start",
        json!({"output": 2}),
    );
    assert!(
        !r["what"].as_str().unwrap().starts_with("First a check"),
        "{r}"
    );
    ok_at(&mut s, c, 10_000, "dspi_measure_stop", json!({}));

    // The stated woofer behind its own 300 Hz low-pass (the demo's chain): the
    // check cannot see its top end, so it stays held at 1 kHz and says why.
    let r = ok_at(
        &mut s,
        c,
        10_000,
        "dspi_measure_rolloff_start",
        json!({"output": 0}),
    );
    assert!(
        r["what"]
            .as_str()
            .unwrap()
            .starts_with("First a check that Out 1"),
        "{r}"
    );
    app_takes(&mut s, &core);
    app_drives(&core, "detect", 300);
    let st = status(&mut s, c, 20_000);
    let l = core.speaker_layout().unwrap().unwrap();
    let check = l.check_of(0).cloned().unwrap();
    assert_eq!(check.verdict, RoleCheckVerdict::Unfinished, "{l:#?}");
    assert!(check.filters_hide_top, "{l:#?}");
    assert!(
        st["request"]["note"]
            .as_str()
            .unwrap_or("")
            .contains("its own low-pass hides its top end")
            || st["detect"]["checks"][0]["verdict"] == json!("unfinished"),
        "{st}"
    );
    let started = app_takes(&mut s, &core);
    assert_eq!(
        started[0]["ticket"]["loop"],
        json!("rolloff"),
        "{started:?}"
    );
    assert!(core.measure_rolloff_state().floor_hz >= 1_000.0, "held");
    ok_at(&mut s, c, 20_000, "dspi_measure_stop", json!({}));
    // Its low-pass taken off, as the note says (music and filters follow the
    // stated role, ROLESFIX): checked again, its top falls, measured as stated.
    ok_at(
        &mut s,
        c,
        20_000,
        "dspi_set_crossover",
        json!({"output": 0, "low_pass": null}),
    );
    let r = ok_at(
        &mut s,
        c,
        20_000,
        "dspi_measure_rolloff_start",
        json!({"output": 0}),
    );
    assert!(
        r["what"]
            .as_str()
            .unwrap()
            .starts_with("First a check that Out 1"),
        "{r}"
    );
    app_takes(&mut s, &core);
    app_drives(&core, "detect", 300);
    status(&mut s, c, 30_000);
    let l = core.speaker_layout().unwrap().unwrap();
    assert!(
        matches!(
            l.check_of(0).map(|c| c.verdict),
            Some(RoleCheckVerdict::TopFalls | RoleCheckVerdict::Silent)
        ),
        "{l:#?}"
    );
    let started = app_takes(&mut s, &core);
    assert_eq!(
        started[0]["ticket"]["loop"],
        json!("rolloff"),
        "{started:?}"
    );
    assert!(core.measure_rolloff_state().floor_hz < 1_000.0);
    ok_at(&mut s, c, 30_000, "dspi_measure_stop", json!({}));
}

#[test]
fn wp_roles_the_tool_is_listed_closed_and_named_as_the_owner_asked() {
    let (set, _id, _core) = demo();
    let (mut s, _) = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "roles-agent", "version": "1"}}}}),
        )
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let t = tools.iter().find(|t| t["name"] == json!(ROLES)).unwrap();
    assert!(!ROLES.contains("speaker_layout"));
    assert_eq!(
        t["inputSchema"]["additionalProperties"],
        json!(false),
        "{t}"
    );
    assert_eq!(t["inputSchema"]["required"], json!(["rows", "user_words"]));
    // An argument the schema does not have is refused before the tool runs.
    let e = err_at(
        &mut s,
        c,
        0,
        ROLES,
        json!({"user_words": "w", "rows": [{"output": 0, "role": "tweeter", "speaker": "L"}], "confirmed": true}),
    );
    assert!(e.contains("confirmed"), "{e}");
}

#[test]
fn wp_roles_a_measurement_across_units_waits_for_the_units_own_check() {
    let (set, id, core) = demo();
    let (mut s, policy) = server(&set, ConfirmMode::NeverAsk);
    policy.set_agent_roles(true);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        ROLES,
        json!({"user_words": "1 woofer, 3 tweeter", "rows": [
            {"output": 0, "role": "woofer", "speaker": "L"},
            {"output": 2, "role": "tweeter", "speaker": "L"}]}),
    );
    let e = err_at(
        &mut s,
        c,
        0,
        "dspi_system_measure_preflight",
        json!({"units": [{"device": id.as_str(), "outputs": [0, 2]}]}),
    );
    assert!(e.contains("not measured across units"), "{e}");
    // A stated tweeter alone needs no check
    let r = call_at(
        &mut s,
        c,
        0,
        "dspi_system_measure_preflight",
        json!({"units": [{"device": id.as_str(), "outputs": [2]}]}),
    );
    assert!(!r.to_string().contains("not measured across units"), "{r}");
    let _ = core;
}

// -- WP REVDFIX -- (REVD B1, B2) the ratchet on the user's own tweeter, and
// the 1 kHz rule on a midrange that is not the user's word

/// Out 3 (from 0: 2) routed, on and unmuted: it plays programme.
fn revdfix_plays(core: &dspi_core::DspiCore, o: u8) {
    core.set_output_enable(o, true).unwrap();
    core.set_output_mute(o, false).unwrap();
    core.set_matrix_route(0, o, true, 0.0, false).unwrap();
}

/// Out 1 woofer, Out 2 midrange, Out 3 as `role`.
fn revdfix_rows(role: &str) -> Value {
    json!([{"output": 0, "role": "woofer", "speaker": "L"},
           {"output": 1, "role": "midrange", "speaker": "L"},
           {"output": 2, "role": role, "speaker": "L"}])
}

const REVDFIX_LOWER: &str = r#"{"family": "linkwitz_riley", "order": 4, "freq_hz": 100}"#;

#[test]
fn wp_revdfix_never_ask_refuses_a_statement_that_demotes_the_users_tweeter() {
    let lower: Value = serde_json::from_str(REVDFIX_LOWER).unwrap();
    for role in ["midrange", "woofer"] {
        let (set, _id, core) = demo();
        // The user's confirmed setup: Out 3 a tweeter, playing.
        layout(&core, true);
        revdfix_plays(&core, 2);
        let (mut s, policy) = server(&set, ConfirmMode::NeverAsk);
        policy.set_agent_roles(true);
        let c = s.connect();
        let e = err_at(
            &mut s,
            c,
            0,
            ROLES,
            json!({"user_words": "out 3 is the woofer now", "what_changed": "the user rewired",
                   "rows": revdfix_rows(role)}),
        );
        assert!(
            e.starts_with("Out 3 is a tweeter in the user's own speaker setup"),
            "{role}: {e}"
        );
        assert_eq!(
            core.speaker_layout().unwrap().unwrap().state(),
            LayoutState::Confirmed,
            "nothing kept"
        );
        // The high-pass still may not come down.
        let r = call_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": lower.clone()}),
        );
        assert_eq!(r["isError"], json!(true), "{role}: {r}");
        // Left out, or stated a tweeter, the table is kept.
        let r = ok_at(
            &mut s,
            c,
            0,
            ROLES,
            json!({"user_words": "1 woofer 2 mid", "what_changed": "the user rewired",
                   "rows": [{"output": 0, "role": "woofer", "speaker": "L"},
                            {"output": 1, "role": "midrange", "speaker": "L"}]}),
        );
        assert_eq!(r["roles_from"], json!("speaker_setup_stated"), "{r}");
        // Still a tweeter to the rules: the lowering is refused.
        let r = call_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": lower.clone()}),
        );
        assert_eq!(r["isError"], json!(true), "{role}: {r}");
        // And a later statement that names it is refused as well: the
        // statement it replaces carried the user's tweeter forward.
        let e = err_at(
            &mut s,
            c,
            0,
            ROLES,
            json!({"user_words": "out 3 is the woofer now", "rows": revdfix_rows(role)}),
        );
        assert!(e.starts_with("Out 3 is a tweeter in the user's own"), "{e}");
    }
}

#[test]
fn wp_revdfix_the_question_shows_the_users_rows_and_their_tweeter_stays_one() {
    let lower: Value = serde_json::from_str(REVDFIX_LOWER).unwrap();
    for role in ["midrange", "woofer"] {
        let (set, _id, core) = demo();
        layout(&core, true);
        revdfix_plays(&core, 2);
        let (mut s, _) = server(&set, ConfirmMode::AskRisky);
        let c = s.connect();
        let cid = held_at(
            &mut s,
            c,
            0,
            ROLES,
            json!({"user_words": "out 3 is the woofer now", "what_changed": "the user rewired",
                   "rows": revdfix_rows(role)}),
        );
        let sheet = s.confirmation(cid).unwrap().to_json();
        let what = sheet["risks"][0]["what"].as_str().unwrap().to_owned();
        let want = format!("It changes your own setup: Out 3 tweeter \u{2192} {role}.");
        assert!(what.contains(&want), "{what}");
        assert!(
            what.contains("A tweeter you set stays a tweeter for agents until you change it in Settings › Speakers."),
            "{what}"
        );
        assert!(
            what.contains("This replaces your confirmed setup"),
            "{what}"
        );
        let r = allow(&mut s, c, cid);
        assert_eq!(r["user_tweeters_kept"], json!([2]), "{r}");
        let l = core.speaker_layout().unwrap().unwrap();
        assert_eq!(l.state(), LayoutState::Stated);
        assert!(l.user_tweeter_held(2));
        assert_eq!(
            dspi_core::trusted_roles(&l).unwrap()[2],
            Some(MeasureRole::High)
        );
        let ov = overview(&mut s, c);
        assert_eq!(ov["outputs"][2]["treated_as_tweeter"], json!(true), "{ov}");
        // The 100 Hz high-pass is refused, not asked: still the user's tweeter.
        let r = call_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": lower.clone()}),
        );
        assert_eq!(r["isError"], json!(true), "{role}: {r}");
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("was a tweeter in the user's own speaker setup"),
            "{r}"
        );
        // The user changes it in Settings › Speakers: their word holds.
        let mut mine = l.clone();
        mine.confirmed_by_user = true;
        core.speaker_layout_set(mine).unwrap();
        let l = core.speaker_layout().unwrap().unwrap();
        assert!(!l.user_tweeter_held(2));
    }
}

#[test]
fn wp_revdfix_never_ask_guards_a_stated_midranges_tweeter_high_pass() {
    let keep = json!({"family": "linkwitz_riley", "order": 4, "freq_hz": 1200});
    let lower = json!({"family": "linkwitz_riley", "order": 4, "freq_hz": 500});
    // Stated (not the user's word): refused as for a woofer.
    for role in ["midrange", "woofer"] {
        let (set, _id, core) = demo();
        revdfix_plays(&core, 2);
        let (mut s, policy) = server(&set, ConfirmMode::NeverAsk);
        policy.set_agent_roles(true);
        let c = s.connect();
        ok_at(
            &mut s,
            c,
            0,
            ROLES,
            json!({"user_words": "w", "rows": revdfix_rows(role)}),
        );
        ok_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": keep.clone()}),
        );
        let r = call_at(
            &mut s,
            c,
            0,
            "dspi_set_crossover",
            json!({"output": 2, "high_pass": lower.clone()}),
        );
        assert_eq!(r["isError"], json!(true), "stated {role}: {r}");
    }
    // The user's own midrange: its 80 Hz rule is the user's word.
    let (set, _id, core) = demo();
    core.speaker_layout_set(dspi_core::SpeakerLayout {
        outputs: [MeasureRole::Low, MeasureRole::Mid, MeasureRole::Mid]
            .iter()
            .enumerate()
            .map(|(o, role)| dspi_core::SpeakerLayoutOutput {
                output: o as u8,
                role: *role,
                speaker: "L".into(),
                ..Default::default()
            })
            .collect(),
        confirmed_by_user: true,
        ..Default::default()
    })
    .unwrap();
    revdfix_plays(&core, 2);
    let (mut s, _) = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok_at(
        &mut s,
        c,
        0,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": keep}),
    );
    ok_at(
        &mut s,
        c,
        0,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": lower}),
    );
}
