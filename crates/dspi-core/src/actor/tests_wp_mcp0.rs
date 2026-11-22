//! WP MCP0: an actor's writes are attributed, the embedder hears of them,
//! one change is reverted by id, the pre-write hook sees every write before
//! it is sent, and the user's drag holds an agent's writes off.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::callbacks::NotifyListener;
use crate::consts::{NOTIFY_CURVES, NOTIFY_HISTORY, NOTIFY_STATE};
use crate::records::NotifyResult;
use crate::{HistorySource, ParamValue};

/// A crossover type (32..63 are the crossover family's).
const XOVER_TYPE: u8 = 33;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    settle(&core);
    core
}

/// Every notification of the open device applied, for `ms` milliseconds;
/// what they raised.
fn gather(core: &DspiCore, ms: u64) -> NotifyResult {
    let mut acc = NotifyResult::default();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        let r = core.process_notifications();
        acc.flags |= r.flags;
        acc.filter_channels |= r.filter_channels;
        std::thread::sleep(Duration::from_millis(5));
    }
    acc
}

fn settle(core: &DspiCore) {
    gather(core, 100);
}

fn gain(core: &DspiCore, o: usize) -> f32 {
    core.lock().state.output_gain_db[o]
}

fn code(e: &DspiError) -> &str {
    match e {
        DspiError::ConfirmationRequired { code, .. } => code,
        _ => "",
    }
}

/// A hook that keeps every proposal it is shown and lets it through.
/// What a [`recorder`] saw: whether each proposal was whole, and its writes.
type Seen = Arc<Mutex<Vec<(bool, Vec<ProposedWrite>)>>>;

fn recorder() -> (WriteHook, Seen) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&seen);
    let hook: WriteHook = Arc::new(move |p: &WriteProposal<'_>| {
        keep.lock().unwrap().push((p.whole, p.writes.to_vec()));
        Ok(())
    });
    (hook, seen)
}

struct Counter(AtomicU32);

impl NotifyListener for Counter {
    fn on_notifications(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn wp_mcp0_a_second_callers_write_raises_state_curves_and_history() {
    let core = demo();
    let woken = Arc::new(Counter(AtomicU32::new(0)));
    core.set_notify_listener(Some(woken.clone()));
    // The app's own write: its echo equals the mirror and raises no STATE.
    core.set_output_gain(0, -2.0).unwrap();
    let own = gather(&core, 300);
    assert_eq!(own.flags & NOTIFY_STATE, 0, "{own:?}");
    assert_ne!(own.flags & NOTIFY_HISTORY, 0, "{own:?}");

    // An agent on another thread, the same core.
    let agent = register_actor("AI · Test agent");
    let woken_before = woken.0.load(Ordering::SeqCst);
    std::thread::scope(|t| {
        t.spawn(|| {
            let _scope = ActorScope::enter(&core, agent, None);
            core.set_filter(
                4,
                0,
                FilterParams {
                    filter_type: FilterType::Peaking.to_raw(),
                    bypass: false,
                    freq: 1_000.0,
                    q: 1.0,
                    gain: -3.0,
                    qp: 0.707,
                },
            )
            .unwrap();
        });
    });
    assert!(
        woken.0.load(Ordering::SeqCst) > woken_before,
        "the scope's end wakes the app"
    );
    let r = core.process_notifications();
    let all = NOTIFY_STATE | NOTIFY_CURVES | NOTIFY_HISTORY;
    assert_eq!(r.flags & all, all, "{r:?}");
    assert_ne!(r.filter_channels & (1 << 4), 0, "channel 4's curve: {r:?}");
    // Told once.
    let again = gather(&core, 200);
    assert_eq!(again.flags & NOTIFY_STATE, 0, "{again:?}");
    core.set_notify_listener(None);
}

#[test]
fn wp_mcp0_history_names_the_actor_and_scopes_are_per_thread_and_core() {
    let core = demo();
    let other = demo();
    let agent = register_actor("AI · Claude\u{7} Code");
    assert_eq!(actor_label(agent).as_deref(), Some("AI · Claude Code"));
    assert_eq!(actor_label(EMBEDDER), None);
    // Review RA F6: format characters go too (a bidi override would make
    // the History read backwards; zero-width ones hide in a label).
    let tricky =
        register_actor("AI \u{202E}edoC\u{202C}\u{200B}\u{2066}·\u{FEFF}\u{E0041} X\u{00AD}");
    assert_eq!(actor_label(tricky).as_deref(), Some("AI edoC· X"));
    assert_eq!(
        actor_label(register_actor("Ärlig agent · 日本 ✓")).as_deref(),
        Some("Ärlig agent · 日本 ✓"),
        "other text stays"
    );
    {
        let _scope = ActorScope::enter(&core, agent, None);
        core.set_output_gain(0, -1.0).unwrap();
        // Another core on this thread is not the agent's.
        other.set_output_gain(0, -1.0).unwrap();
        // Another thread is not the agent's.
        std::thread::scope(|t| {
            t.spawn(|| core.set_output_gain(1, -1.0).unwrap());
        });
    }
    core.set_output_gain(2, -1.0).unwrap();
    let h = core.history(3);
    assert_eq!(
        h.iter()
            .map(|e| (e.indices[0], e.actor))
            .collect::<Vec<_>>(),
        vec![(2, EMBEDDER), (1, EMBEDDER), (0, agent)]
    );
    assert_eq!(h[2].actor_label.as_deref(), Some("AI · Claude Code"));
    assert_eq!(h[0].actor_label, None);
    assert_eq!(other.history(1)[0].actor, EMBEDDER);
    let ids: Vec<u64> = h.iter().map(|e| e.id).collect();
    assert!(ids.iter().all(|i| *i > 0) && ids[0] != ids[1] && ids[1] != ids[2]);
}

#[test]
fn wp_mcp0_an_agents_write_never_joins_the_users_group() {
    let core = demo();
    let agent = register_actor("agent");
    core.begin_group("Clear all".into());
    core.set_output_gain(0, -1.0).unwrap();
    {
        let _scope = ActorScope::enter(&core, agent, None);
        core.set_output_gain(1, -1.0).unwrap();
    }
    core.set_output_gain(2, -1.0).unwrap();
    core.end_group();
    let h = core.history(10);
    let agents: Vec<_> = h.iter().filter(|e| e.actor == agent).collect();
    assert_eq!(agents.len(), 1, "{h:?}");
    assert_eq!(agents[0].label, None, "not the user's label");
    assert_eq!(agents[0].indices, vec![1]);
    // Undo of the user's newest step leaves the agent's value.
    core.undo().unwrap();
    assert_eq!(gain(&core, 1), -1.0);
    assert_eq!(gain(&core, 2), 0.0);
}

#[test]
fn wp_mcp0_revert_entry_restores_only_an_unchanged_value() {
    let core = demo();
    let agent = register_actor("agent");
    let scope = ActorScope::enter(&core, agent, None);
    core.set_output_gain(0, -3.0).unwrap();
    let mine = core.history(1)[0].id;
    core.set_output_gain(1, -4.0).unwrap();
    let second = core.history(1)[0].id;
    drop(scope);
    // The user changes something else meanwhile.
    core.set_output_delay(2, 1.5).unwrap();

    // Reverted out of order: the newer change and the user's stay.
    let _scope = ActorScope::enter(&core, agent, None);
    let m = core.revert_entry(mine).unwrap();
    assert_eq!(m.entry.as_ref().map(|e| e.id), Some(mine), "{m:?}");
    assert_eq!(gain(&core, 0), 0.0);
    assert_eq!(gain(&core, 1), -4.0);
    assert_eq!(core.lock().state.output_delay_ms[2], 1.5);
    let newest = &core.history(1)[0];
    assert_eq!((newest.source, newest.actor), (HistorySource::Undo, agent));
    // Gone from the stacks: neither again by id nor by undo.
    let again = core.revert_entry(mine).unwrap();
    assert!(again.entry.is_none(), "{again:?}");
    assert!(again.note.is_some());
    drop(_scope);

    // The user overwrites the agent's other value: a conflict, nothing sent.
    core.set_output_gain(1, -6.0).unwrap();
    let _scope = ActorScope::enter(&core, agent, None);
    let c = core.revert_entry(second).unwrap();
    assert!(c.entry.is_none(), "{c:?}");
    assert_eq!(c.skipped.len(), 1, "{c:?}");
    assert!(c.note.is_some());
    assert_eq!(gain(&core, 1), -6.0);

    let e = core.revert_entry(u64::MAX).unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
}

#[test]
fn wp_mcp0_an_agent_cannot_revert_the_users_change_but_the_app_can_revert_the_agents() {
    let core = demo();
    let agent = register_actor("agent");
    let other = register_actor("another agent");
    core.set_output_gain(0, -3.0).unwrap();
    let users = core.history(1)[0].id;
    {
        let _scope = ActorScope::enter(&core, agent, None);
        let e = core.revert_entry(users).unwrap_err();
        assert_eq!(code(&e), NOT_OWN_ENTRY, "{e:?}");
        core.set_output_gain(1, -2.0).unwrap();
    }
    assert_eq!(gain(&core, 0), -3.0);
    let agents = core.history(1)[0].id;
    {
        let _scope = ActorScope::enter(&core, other, None);
        let e = core.revert_entry(agents).unwrap_err();
        assert_eq!(code(&e), NOT_OWN_ENTRY, "{e:?}");
    }
    // The app (its activity log's per-row Undo) may.
    let m = core.revert_entry(agents).unwrap();
    assert!(m.entry.is_some(), "{m:?}");
    assert_eq!(gain(&core, 1), 0.0);
}

#[test]
fn wp_mcp0_the_hook_sees_before_and_after_for_setters_write_param_and_recall() {
    let core = demo();
    core.set_output_gain(0, -1.0).unwrap();
    let snap = core
        .snapshot_capture_document("before".into(), String::new())
        .unwrap();
    let agent = register_actor("agent");
    // The first output's wire channel.
    let out = core.lock().state.num_input_channels;
    let (hook, seen) = recorder();
    let _scope = ActorScope::enter(&core, agent, Some(hook));

    // A typed setter.
    core.set_output_gain(0, -4.0).unwrap();
    // write_param.
    core.write_param("out.delay".into(), vec![1], ParamValue::Float(2.0))
        .unwrap();
    // A whole band: each field it changes.
    core.set_crossover(
        out,
        0,
        FilterParams {
            filter_type: XOVER_TYPE,
            bypass: false,
            freq: 2_000.0,
            q: 0.707,
            gain: 0.0,
            qp: 0.707,
        },
    )
    .unwrap();
    // An effect.
    core.set_loudness(true).unwrap();
    // A snapshot recall: the whole state, as one proposal.
    core.snapshot_recall_document(snap).unwrap();

    let seen = seen.lock().unwrap();
    let find = |path: &str, ix: &[u8]| -> Vec<(bool, ProposedWrite)> {
        seen.iter()
            .flat_map(|(whole, w)| w.iter().map(move |w| (*whole, w.clone())))
            .filter(|(_, w)| w.path == path && w.indices == ix)
            .collect()
    };
    let g = find("out.gain", &[0]);
    assert!(!g[0].0, "a setter's write is not whole");
    assert_eq!(g[0].1.before, Some(ParamValue::Float(-1.0)));
    assert_eq!(g[0].1.after, ParamValue::Float(-4.0));
    // The recall puts it back: before −4, after −1, as part of a whole.
    let back = g.iter().find(|(whole, _)| *whole).expect("the recall's");
    assert_eq!(back.1.before, Some(ParamValue::Float(-4.0)));
    assert_eq!(back.1.after, ParamValue::Float(-1.0));

    let d = find("out.delay", &[1]);
    assert_eq!(d[0].1.before, Some(ParamValue::Float(0.0)));
    assert_eq!(d[0].1.after, ParamValue::Float(2.0));
    // The crossover's band is wire band 20.
    let t = find("eq.type", &[out, 20]);
    assert_eq!(t[0].1.after, ParamValue::Choice(XOVER_TYPE));
    let f = find("eq.freq", &[out, 20]);
    assert_eq!(f[0].1.after, ParamValue::Float(2_000.0));
    let l = find("loud.on", &[]);
    assert_eq!(l[0].1.before, Some(ParamValue::Bool(false)));
    assert_eq!(l[0].1.after, ParamValue::Bool(true));
    // The recall's proposal is whole and names every value it moves back.
    assert!(
        find("loud.on", &[])
            .iter()
            .any(|(whole, w)| *whole && w.after == ParamValue::Bool(false))
    );
}

#[test]
fn wp_mcp0_a_refusing_hook_sends_nothing() {
    let core = demo();
    let agent = register_actor("agent");
    let hook: WriteHook = Arc::new(|p: &WriteProposal<'_>| {
        let louder = p.writes.iter().any(|w| match (&w.before, &w.after) {
            (Some(ParamValue::Float(b)), ParamValue::Float(a)) => a > b,
            _ => false,
        });
        if louder {
            return Err(DspiError::ConfirmationRequired {
                code: "agent_confirm".into(),
                msg: "louder".into(),
            });
        }
        Ok(())
    });
    let len = core.history(u32::MAX).len();
    let _scope = ActorScope::enter(&core, agent, Some(hook));
    let e = core.set_output_gain(0, 3.0).unwrap_err();
    assert_eq!(code(&e), "agent_confirm");
    assert_eq!(gain(&core, 0), 0.0);
    assert_eq!(core.history(u32::MAX).len(), len, "nothing journalled");
    // Quieter passes.
    core.set_output_gain(0, -3.0).unwrap();
    assert_eq!(gain(&core, 0), -3.0);
    // Reverting the cut is a boost: the hook judges replays too.
    let id = core.history(1)[0].id;
    let e = core.revert_entry(id).unwrap_err();
    assert_eq!(code(&e), "agent_confirm");
    assert_eq!(gain(&core, 0), -3.0);
}

#[test]
fn wp_mcp0_a_write_while_the_user_drags_is_refused_as_retryable() {
    let core = demo();
    let agent = register_actor("agent");
    core.set_dragging(true);
    core.set_output_gain(0, -1.0).unwrap();
    {
        let _scope = ActorScope::enter(&core, agent, None);
        let e = core.set_output_gain(1, -2.0).unwrap_err();
        assert_eq!(code(&e), USER_EDITING, "{e:?}");
        let e = core
            .write_param("out.gain".into(), vec![1], ParamValue::Float(-2.0))
            .unwrap_err();
        assert_eq!(code(&e), USER_EDITING, "{e:?}");
        assert_eq!(gain(&core, 1), 0.0);
    }
    // The user's own drag goes on.
    core.set_output_gain(0, -1.5).unwrap();
    core.set_dragging(false);
    let _scope = ActorScope::enter(&core, agent, None);
    core.set_output_gain(1, -2.0).unwrap();
    assert_eq!(gain(&core, 1), -2.0);
}
