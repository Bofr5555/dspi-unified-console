//! WP FFIX: the open bugs of the first real-agent function test (FUNC1).
//! Bug 1: the tweeter rule judges the state a write leaves, on every write
//! path; bug 2: an agent's preset load runs silenced on a device with a
//! confirmed tweeter.

use std::path::PathBuf;
use std::sync::Arc;

use dspi_core::measure::MeasureRole;
use dspi_core::{
    CoreSet, CoreSetConfig, DeviceTarget, DspiCore, FilterParams, SpeakerLayout,
    SpeakerLayoutOutput,
};
use serde_json::{Value, json};

use crate::policy::{self, AgentPolicy, ConfirmMode, Judging, Verdict, WriteKey};
use crate::rate::RateLimit;
use crate::{ClientId, Server, ServerConfig};

const MODERN: &str = "2026-07-28";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("ffix-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn call(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    let params = json!({"name": tool, "arguments": args, "_meta": {
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "ffix-agent", "version": "1"},
    }});
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params}),
        )
        .unwrap();
    r.get("result").cloned().unwrap_or_else(|| panic!("{r}"))
}

pub(crate) fn ok(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    let r = call(s, c, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    assert_ne!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{tool} was held: {r}"
    );
    r["structuredContent"].clone()
}

pub(crate) fn err(s: &mut Server, c: ClientId, tool: &str, args: Value) -> String {
    let r = call(s, c, tool, args);
    assert_eq!(r["isError"], json!(true), "{tool}: {r}");
    r["content"][0]["text"].as_str().unwrap().to_owned()
}

pub(crate) fn server_of(set: &Arc<CoreSet>, mode: ConfirmMode) -> Server {
    let limit = RateLimit {
        burst: 10_000,
        per_s: 1.0,
    };
    Server::new(
        Arc::clone(set),
        ServerConfig {
            policy: Arc::new(AgentPolicy::new(mode)),
            read_limit: limit,
            write_limit: limit,
            measuring: true,
            ..ServerConfig::default()
        },
    )
}

pub(crate) fn demo(name: &str) -> (Arc<CoreSet>, Arc<DspiCore>) {
    let dir = scratch(name);
    let set = CoreSet::new();
    set.set_config(CoreSetConfig {
        snapshot_dir: dir.join("snap").to_string_lossy().into_owned(),
        measure_dir: dir.join("measure").to_string_lossy().into_owned(),
        ..CoreSetConfig::default()
    })
    .unwrap();
    let id = set.open(DeviceTarget::Demo).unwrap();
    let core = set.core(&id).unwrap();
    core.fetch_all().unwrap();
    (set, core)
}

/// Output 1 the user's confirmed tweeter, output 2 a midrange, the rest full.
fn confirm(core: &DspiCore) {
    let outputs = (0..core.get_state().num_output_channels)
        .map(|n| SpeakerLayoutOutput {
            output: n,
            role: match n {
                1 => MeasureRole::High,
                2 => MeasureRole::Mid,
                _ => MeasureRole::Full,
            },
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

fn n_in(core: &DspiCore) -> u8 {
    core.get_state().num_input_channels
}

fn hp(filter_type: u8, hz: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq: hz,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    }
}

const FLAT: FilterParams = policy::FLAT;
const LR4_HP: u8 = 35;

/// Output `o` of `core` as the user left it: no filters, no routes, on and
/// unmuted (set by the user, unjudged).
fn blank(core: &DspiCore, o: u8) {
    let ch = n_in(core) + o;
    for x in 0..4 {
        core.set_crossover(
            ch,
            x,
            FilterParams {
                filter_type: 0,
                ..FLAT
            },
        )
        .unwrap();
    }
    for b in 0..10 {
        core.set_filter(
            ch,
            b,
            FilterParams {
                filter_type: 0,
                ..FLAT
            },
        )
        .unwrap();
    }
    for i in 0..n_in(core) {
        core.set_matrix_route(i, o, false, 0.0, false).unwrap();
    }
    core.set_output_enable(o, true).unwrap();
    core.set_output_mute(o, false).unwrap();
}

fn routed(core: &DspiCore, i: u8, o: u8) -> bool {
    core.get_state().matrix_routing[usize::from(i)][usize::from(o)]
}

fn crossover_hp(hz: u32, family: &str, order: u8) -> Value {
    json!({"family": family, "order": order, "freq_hz": hz})
}

// ------------------------------------------------------------ bug 1

#[test]
fn wp_ffix_a_tweeter_with_no_highpass_is_never_fed_under_any_setting() {
    for mode in [
        ConfirmMode::NeverAsk,
        ConfirmMode::AskRisky,
        ConfirmMode::AskEverything,
    ] {
        let (set, core) = demo(&format!("feed-{}", mode.as_str()));
        blank(&core, 1);
        confirm(&core);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        // A route to it, refused, never asked.
        let e = err(
            &mut s,
            c,
            "dspi_set_route",
            json!({"input": 0, "output": 1, "on": true, "gain_db": 0}),
        );
        assert!(
            e.contains("output 1 (Out 2 in the app) drives a tweeter")
                && e.contains("1000 Hz or above")
                && e.contains("no such high-pass"),
            "{mode:?}: {e}"
        );
        assert!(s.waiting_confirmations().is_empty(), "{mode:?}");
        assert!(!routed(&core, 0, 1), "{mode:?}");
        // With its high-pass first, it passes.
        if mode == ConfirmMode::NeverAsk {
            ok(
                &mut s,
                c,
                "dspi_set_crossover",
                json!({"output": 1, "high_pass": crossover_hp(2_000, "linkwitz_riley", 4)}),
            );
            ok(
                &mut s,
                c,
                "dspi_set_route",
                json!({"input": 0, "output": 1, "on": true, "gain_db": 0}),
            );
            assert!(routed(&core, 0, 1));
        }
    }
}

#[test]
fn wp_ffix_a_highpass_below_the_floor_or_too_shallow_is_refused_where_there_was_none() {
    let (set, core) = demo("weak");
    blank(&core, 1);
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": crossover_hp(500, "linkwitz_riley", 4)}),
    );
    assert!(e.contains("24 dB/octave high-pass at 500 Hz"), "{e}");
    let e = err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": crossover_hp(2_000, "butterworth", 1)}),
    );
    assert!(e.contains("6 dB/octave high-pass at 2000 Hz"), "{e}");
    let x = core.get_state().xover[usize::from(n_in(&core) + 1)][0];
    assert_eq!(x.filter_type, 0, "nothing was changed");
    // The midrange's floor is 80 Hz.
    blank(&core, 2);
    confirm(&core);
    err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": crossover_hp(50, "linkwitz_riley", 4)}),
    );
    ok(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 2, "high_pass": crossover_hp(300, "linkwitz_riley", 4)}),
    );
}

#[test]
fn wp_ffix_switching_a_tweeter_on_or_unmuting_it_is_judged_on_the_result() {
    let (set, core) = demo("switch");
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    core.set_output_enable(1, false).unwrap();
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = err(
        &mut s,
        c,
        "dspi_set_output_enabled",
        json!({"output": 1, "enabled": true}),
    );
    assert!(e.contains("play programme"), "{e}");
    assert!(!core.get_state().output_enabled[1]);
    core.set_output_enable(1, true).unwrap();
    core.set_output_mute(1, true).unwrap();
    err(
        &mut s,
        c,
        "dspi_set_mute",
        json!({"output": 1, "muted": false}),
    );
    assert!(core.get_state().output_muted[1]);
    // One batch that protects it first passes whole.
    ok(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [
            {"op": "set_mute", "output": 1, "muted": false},
            {"op": "set_crossover", "output": 1, "high_pass": crossover_hp(2_500, "linkwitz_riley", 4)},
        ]}),
    );
    assert!(!core.get_state().output_muted[1]);
    // Now protected and playing: taking the high-pass away is refused.
    let e = err(
        &mut s,
        c,
        "dspi_set_crossover",
        json!({"output": 1, "high_pass": null}),
    );
    assert!(
        e.contains("removes or weakens") || e.contains("lowers"),
        "{e}"
    );
}

#[test]
fn wp_ffix_a_boost_below_the_floor_counts_by_what_reaches_the_tweeter() {
    let (set, core) = demo("boost");
    blank(&core, 1);
    let ch = n_in(&core) + 1;
    core.set_crossover(ch, 0, hp(LR4_HP, 1_000.0)).unwrap();
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    // +20 dB low shelf at 800 Hz on a 1 kHz LR4: above the 12 dB/octave line.
    let e = err(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"output": 1, "slot": 0, "type": "low_shelf", "freq_hz": 800, "gain_db": 20}),
    );
    assert!(e.contains("boosts it below 1000 Hz"), "{e}");
    // A treble cut, and a bass shelf on the input (behind the high-pass), pass.
    ok(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"output": 1, "slot": 0, "type": "peaking", "freq_hz": 5000, "gain_db": -3, "q": 1}),
    );
    ok(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"input": 0, "slot": 0, "type": "low_shelf", "freq_hz": 80, "gain_db": 6}),
    );
    // With no high-pass at all, a +20 dB band at 300 Hz never passes.
    blank(&core, 3);
    let outputs = (0..core.get_state().num_output_channels)
        .map(|n| SpeakerLayoutOutput {
            output: n,
            role: if n == 3 {
                MeasureRole::High
            } else {
                MeasureRole::Full
            },
            speaker: "R".into(),
            group: 1,
        })
        .collect();
    core.speaker_layout_set(SpeakerLayout {
        outputs,
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    })
    .unwrap();
    core.set_matrix_route(1, 3, true, 0.0, false).unwrap();
    err(
        &mut s,
        c,
        "dspi_set_peq_band",
        json!({"output": 3, "slot": 0, "type": "peaking", "freq_hz": 300, "gain_db": 20, "q": 1}),
    );
}

#[test]
fn wp_ffix_an_output_the_user_left_unprotected_may_be_made_no_worse() {
    let (set, core) = demo("already");
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, -6.0, false).unwrap();
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    // A quieter route and a mute pass: they do not make it worse.
    ok(
        &mut s,
        c,
        "dspi_set_route",
        json!({"input": 0, "output": 1, "on": true, "gain_db": -12}),
    );
    // Another input, or a louder route, is refused.
    let e = err(
        &mut s,
        c,
        "dspi_set_route",
        json!({"input": 1, "output": 1, "on": true, "gain_db": -12}),
    );
    assert!(e.contains("feeds it more"), "{e}");
    err(
        &mut s,
        c,
        "dspi_set_route",
        json!({"input": 0, "output": 1, "on": true, "gain_db": 0}),
    );
    ok(
        &mut s,
        c,
        "dspi_set_mute",
        json!({"output": 1, "muted": true}),
    );
}

#[test]
fn wp_ffix_the_limits_profile_raises_the_floor() {
    let (_set, core) = demo("floor");
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    let core_state = core.get_state();
    let ch = core_state.num_input_channels + 1;
    let write = |hz: f32| {
        vec![WriteKey::new(
            "eq.band",
            &[ch, 20],
            dspi_core::ParamValue::Bytes(policy::band_bytes(&hp(LR4_HP, hz))),
        )]
    };
    let mut j = Judging {
        mode: Some(ConfirmMode::NeverAsk),
        roles: vec![None, Some(MeasureRole::High)],
        ..Judging::default()
    };
    assert_eq!(
        policy::judge(&core_state, &write(1_500.0), &j),
        Verdict::Pass
    );
    j.floors = vec![(1, 2_500.0, 24)];
    let Verdict::Refuse(why) = policy::judge(&core_state, &write(1_500.0), &j) else {
        panic!("a 1.5 kHz high-pass under a 2.5 kHz profile floor passed");
    };
    assert!(why.contains("2500 Hz or above, 24 dB/octave"), "{why}");
    assert_eq!(
        policy::judge(&core_state, &write(2_500.0), &j),
        Verdict::Pass
    );
    // A profile never lowers the user's 1 kHz.
    j.floors = vec![(1, 300.0, 6)];
    assert!(matches!(
        policy::judge(&core_state, &write(500.0), &j),
        Verdict::Refuse(_)
    ));
}

// ------------------------------------------------------------ bug 2

/// The master-volume writes in `core`'s history since `n` entries.
fn master_writes(core: &DspiCore) -> Vec<f32> {
    core.history(200)
        .iter()
        .filter(|e| e.path == "vol.master")
        .filter_map(|e| match e.after {
            dspi_core::ParamValue::Float(f) => Some(f),
            _ => None,
        })
        .collect()
}

#[test]
fn wp_ffix_an_agents_preset_load_runs_silenced_and_comes_back() {
    let (set, core) = demo("load");
    // Preset 1: the tweeter routed with no high-pass (the user's own).
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
    core.save_preset_confirmed(1, Some(token)).unwrap();
    // Live: protected.
    core.set_crossover(n_in(&core) + 1, 0, hp(LR4_HP, 2_000.0))
        .unwrap();
    core.set_master_volume(-10.0).unwrap();
    confirm(&core);
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    let e = err(&mut s, c, "dspi_preset_load", json!({"slot": 1}));
    assert!(
        e.contains("drives a tweeter") && e.contains("put back"),
        "{e}"
    );
    let st = core.get_state();
    assert_eq!(
        st.xover[usize::from(n_in(&core) + 1)][0].filter_type,
        LR4_HP
    );
    assert!(
        (st.master_volume_db - -10.0).abs() < 0.01,
        "{}",
        st.master_volume_db
    );
    // The load ran at the guard level.
    assert!(master_writes(&core).contains(&-127.0) || st.master_volume_mode != 0);
    // A preset that keeps it protected loads, and the volume comes back.
    let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
    core.save_preset_confirmed(2, Some(token)).unwrap();
    ok(&mut s, c, "dspi_preset_load", json!({"slot": 2}));
    assert!((core.get_state().master_volume_db - -10.0).abs() < 0.01);
}

// ------------------------------------------------------------ bug 5

/// A tools/call over HTTP with `token`; (status, body, seconds it took).
fn http_call(port: u16, token: &str, tool: &str, args: Value) -> (u16, Value, f64) {
    use std::io::{Read, Write};
    let body = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "ffix-agent", "version": "1"}}}})
    .to_string();
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Authorization: Bearer {token}\r\nMCP-Protocol-Version: {MODERN}\r\nMcp-Method: tools/call\r\n\
         Mcp-Name: {tool}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let began = std::time::Instant::now();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(40)))
        .unwrap();
    s.write_all(head.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |p| p.1);
    (
        status,
        serde_json::from_str(body).unwrap_or(Value::Null),
        began.elapsed().as_secs_f64(),
    )
}

#[test]
fn wp_ffix_a_status_call_with_wait_s_waits_over_http_without_holding_the_server() {
    use crate::http::{HttpConfig, HttpLimits, HttpServer};
    let (set, _core) = demo("poll");
    let http = Arc::new(
        HttpServer::start(
            server_of(&set, ConfirmMode::NeverAsk),
            HttpConfig {
                port: 0,
                store_dir: None,
                limits: HttpLimits::default(),
            },
            None,
        )
        .unwrap(),
    );
    let token = http.add_client("Claude Code").unwrap().token;
    let port = http.port();
    let (st, body, secs) = http_call(port, &token, "dspi_measure_status", json!({}));
    assert_eq!(st, 200, "{body}");
    assert!(secs < 1.0, "no wait asked: {secs}");
    // Nothing changes: the answer comes after the wait, and meanwhile another
    // call is answered at once.
    let waiter = {
        let token = token.clone();
        std::thread::spawn(move || {
            http_call(port, &token, "dspi_measure_status", json!({"wait_s": 2}))
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (_, _, quick) = http_call(port, &token, "dspi_get_overview", json!({}));
    assert!(quick < 1.0, "the server is not held by the wait: {quick}");
    let (st, body, secs) = waiter.join().unwrap();
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    assert!((1.9..4.0).contains(&secs), "{secs}");
    http.stop();
}

// ------------------------------------------------------- bugs 3, 9, 15

#[test]
fn wp_ffix_a_filed_measurement_says_its_arrivals_and_the_highpass_it_includes() {
    use crate::tools::results;
    let doc = json!({"measurement": {
        "stimulus": {"kind": "walked"},
        "capture": {"timingReliable": true},
        "outputs": [
            {"output": 0, "role": "low", "speaker": "L", "arrivalMs": 5.0,
             "chain": {"peq": [], "crossover": [], "delayMs": 0.0}},
            {"output": 1, "role": "high", "speaker": "L", "arrivalMs": 5.1,
             "chain": {"peq": [], "crossover": [{"filterType": 35, "bypass": false, "freq": 2000.0}], "delayMs": 0.0}},
            {"output": 2, "role": "low", "speaker": "R", "arrivalMs": 6.25,
             "chain": {"peq": [], "crossover": [], "delayMs": 0.5}},
            {"output": 4, "role": "sub", "speaker": "Sub", "arrivalMs": 9.0,
             "chain": {"peq": [], "crossover": [], "delayMs": 0.0}}]}})
    .to_string();
    let s = results::summary(&doc).unwrap();
    assert_eq!(s["outputs"][1]["arrival_ms"], json!(5.1));
    assert!(
        s["outputs"][1]["highpass"][0]
            .as_str()
            .unwrap()
            .starts_with("24 dB/octave high-pass at 2000 Hz")
    );
    let note = results::highpass_note(
        &json!({"crossover": [{"filterType": 35, "bypass": false, "freq": 2000.0}]}),
    );
    assert!(note.unwrap().contains("the filter's, not the driver's own"));
    assert!(results::highpass_note(&json!({"crossover": []})).is_none());
    // R arrives 0.75 ms after L (6.25 - 0.5 measured delay + 0 now): L and its
    // tweeter get 0.75 ms more; the sub is left to its crossover.
    let (_set, core) = demo("align");
    for o in 0..5 {
        core.set_output_delay(o, 0.0).unwrap();
    }
    let a = results::alignment(&doc, &core.get_state(), core.max_delay_ms()).unwrap();
    assert_eq!(
        a["changes"],
        json!([{"op": "set_delay", "output": 0, "delay_ms": 0.75},
               {"op": "set_delay", "output": 1, "delay_ms": 0.75}]),
        "{a}"
    );
    assert!(a["note"].as_str().unwrap().contains("Sub"), "{a}");
}

// ------------------------------------------------------------ bug 15

#[test]
fn wp_ffix_a_preset_save_offers_no_undo_and_apply_changes_points_to_the_limiter() {
    let (set, _core) = demo("save");
    let mut s = server_of(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(&mut s, c, "dspi_preset_save", json!({"slot": 1}));
    let e = s
        .activity(10)
        .into_iter()
        .rev()
        .find(|e| e.tool == "dspi_preset_save")
        .unwrap();
    assert!(e.journal_ids.is_empty(), "nothing Undo can reverse: {e:?}");
    // An undoable change still lists its entry.
    ok(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 0, "gain_db": -3}),
    );
    let e = s
        .activity(10)
        .into_iter()
        .rev()
        .find(|e| e.tool == "dspi_set_gain")
        .unwrap();
    assert_eq!(e.journal_ids.len(), 1, "{e:?}");
    let err = err(
        &mut s,
        c,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_limiter", "output": 0}]}),
    );
    assert!(err.contains("dspi_set_limiter"), "{err}");
}
