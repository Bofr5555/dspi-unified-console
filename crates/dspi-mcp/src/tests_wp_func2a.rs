//! WP FUNC2a: what the function test's stories found in the MCP server.
//!
//! S4 ("make the tweeters go lower"): `dspi_set_crossover`'s text still said
//! that lowering a confirmed tweeter's high-pass is refused, so three agents
//! out of three would not move one at all. The rule (FFIX) is on the state a
//! write leaves: a confirmed tweeter's high-pass may move, lower too, while it
//! stays at the floor or above and steep enough.

use dspi_core::FilterParams;
use dspi_core::measure::MeasureRole;
use dspi_core::speaker_layout::{SpeakerLayout, SpeakerLayoutOutput};
use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{demo, err, ok, server_of};

const LR4_HP: u8 = 35;

/// Output 1 the user's confirmed tweeter, LR4 at 2.5 kHz and routed, as the
/// user left it.
fn tweeter_at_2500(core: &dspi_core::DspiCore) {
    let outputs = (0..core.get_state().num_output_channels)
        .map(|n| SpeakerLayoutOutput {
            output: n,
            role: if n == 1 {
                MeasureRole::High
            } else {
                MeasureRole::Full
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
    let ch = core.get_state().num_input_channels + 1;
    core.set_crossover(
        ch,
        0,
        FilterParams {
            filter_type: LR4_HP,
            bypass: false,
            freq: 2500.0,
            q: 0.707,
            gain: 0.0,
            qp: 0.707,
        },
    )
    .unwrap();
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    core.set_output_enable(1, true).unwrap();
    core.set_output_mute(1, false).unwrap();
}

#[test]
fn wp_func2a_a_confirmed_tweeters_highpass_may_go_lower_down_to_the_floor() {
    for mode in [
        ConfirmMode::NeverAsk,
        ConfirmMode::AskRisky,
        ConfirmMode::AskEverything,
    ] {
        let (set, core) = demo(&format!("func2a-lower-{mode:?}"));
        tweeter_at_2500(&core);
        let mut s = server_of(&set, mode);
        let c = s.connect();
        let hp = |hz: u32| json!({"family": "linkwitz_riley", "order": 4, "freq_hz": hz});
        let e = err(
            &mut s,
            c,
            "dspi_set_crossover",
            json!({"output": 1, "high_pass": hp(900)}),
        );
        assert!(e.contains("1000 Hz"), "{mode:?}: {e}");
        if mode == ConfirmMode::AskEverything {
            // Everything is asked there; the rule is the same
            continue;
        }
        ok(
            &mut s,
            c,
            "dspi_set_crossover",
            json!({"output": 1, "high_pass": hp(1800)}),
        );
        let ch = usize::from(core.get_state().num_input_channels) + 1;
        let x = core.get_state().xover[ch][0];
        assert_eq!((x.filter_type, x.freq), (LR4_HP, 1800.0), "{mode:?}");
    }
}

#[test]
fn wp_func2a_the_crossover_tool_says_a_confirmed_tweeter_may_go_lower() {
    let (set, _core) = demo("func2a-text");
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let params = json!({"_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "func2a-agent", "version": "1"},
    }});
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": params}),
        )
        .unwrap();
    let text = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "dspi_set_crossover")
        .and_then(|t| t["description"].as_str())
        .unwrap()
        .to_owned();
    assert!(text.contains("also lower"), "{text}");
    assert!(!text.contains("refused on an output the user's speaker setup calls a tweeter"));
}
