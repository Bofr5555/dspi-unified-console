//! WP HUBADMIN: the Installer page's copy and the hub admin records without a
//! hub. The calls against a real hub are in
//! `crates/dspi-hub/tests/wp_hubadmin_admin.rs`.

use std::sync::Mutex;

use serde_json::json;

use super::*;

#[test]
fn wp_hubadmin_no_device_edits_nothing() {
    let core = DspiCore::new();
    let t = core.installer_target();
    assert_eq!(t.copy, COPY_NONE);
    assert!(!t.editable);
    assert_eq!(t.read_only_key, "disconnected");
}

#[test]
fn wp_hubadmin_a_direct_device_edits_the_consoles_copy() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let t = core.installer_target();
    assert_eq!(t.copy, COPY_CONSOLE, "{t:?}");
    assert!(t.editable);
    assert_eq!(t.notice_key, NOTICE_CONSOLE_COPY);
    assert!(t.read_only_key.is_empty());
    assert!(!t.serial.is_empty());
    assert!(crate::notice_text::has_row(&t.notice_key));
}

#[test]
fn wp_hubadmin_hub_calls_off_a_hub_say_so() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let e = core.hub_limits_get().unwrap_err();
    assert_eq!(e.notice_key(), NOTICE_NO_LIMITS, "{e:?}");
    // The arguments are checked before anything else.
    let e = core.hub_pin_set("12".into()).unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let e = core.hub_limits_set("not json".into()).unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let e = core
        .hub_mode_pin("guest".into(), String::new())
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    let e = core
        .hub_mode_pin("control".into(), "expert".into())
        .unwrap_err();
    assert!(matches!(e, DspiError::InvalidArgument { .. }), "{e:?}");
    #[cfg(feature = "link")]
    {
        let doc = dspi_session::limits::LimitsProfile::default().to_document();
        let e = core.hub_limits_set(doc).unwrap_err();
        assert_eq!(e.notice_key(), NOTICE_NO_LIMITS, "{e:?}");
        assert_eq!(
            core.hub_pin_clear().unwrap_err().notice_key(),
            NOTICE_NO_LIMITS
        );
        assert_eq!(
            core.hub_mode_pin("control".into(), "simple".into())
                .unwrap_err()
                .notice_key(),
            NOTICE_NO_LIMITS
        );
    }
}

#[test]
fn wp_hubadmin_every_page_notice_has_a_row() {
    for key in [
        NOTICE_HUB_COPY,
        NOTICE_CONSOLE_COPY,
        NOTICE_HUB_ROLE,
        NOTICE_NO_LIMITS,
        NOTICE_HUB_RECOVERY,
    ] {
        assert!(crate::notice_text::has_row(key), "{key}");
    }
    // The Admin-only refusal reads as the hub's role row's neighbour: `denied`.
    assert_eq!(DspiError::Denied.notice_key(), "denied");
}

#[test]
fn wp_hubadmin_the_hubs_events_are_read_and_others_ignored() {
    let e = HubAdminEvent::from_hub(
        "limits.held",
        &json!({"t": "limits.held", "serial": "S1", "rows": ["out.gain[0] 6 -> 3 dB", 7]}),
    )
    .unwrap();
    assert_eq!(e.kind, "limits.held");
    assert_eq!(e.serial, "S1");
    assert_eq!(e.rows, ["out.gain[0] 6 -> 3 dB"]);
    let e =
        HubAdminEvent::from_hub("mode.changed", &json!({"role": "control", "mode": null})).unwrap();
    assert_eq!((e.role.as_str(), e.mode.as_str()), ("control", ""));
    assert!(HubAdminEvent::from_hub("system.changed", &json!({})).is_none());
    assert!(HubAdminEvent::from_hub("apply.reverted", &json!({})).is_none());
}

#[test]
fn wp_hubadmin_the_hubs_answers_read_as_one_record() {
    let got = json!({
        "serial": "S1", "handle": 1,
        "profile": {"outputs": [{"output": 0, "maxGainDb": 3.0}], "lockedPages": ["eq"]},
        "pin_set": true, "locked": true, "unlocked": true,
        "held": ["a"], "wrong_tries": 2, "retry_in_ms": 0
    });
    let pinned = json!({"role": "admin", "mode": null,
        "pins": {"viewer": "simple", "control": null, "admin": "installer"}});
    let st = state_from(&got, &pinned, "admin".into());
    assert!(st.can_edit && st.pin_set && st.locked && st.unlocked);
    assert_eq!(st.wrong_tries, 2);
    let p = dspi_session::limits::from_value(
        &serde_json::from_str::<Value>(&st.profile_document).unwrap(),
    )
    .unwrap();
    assert_eq!(p.locked_pages, ["eq"]);
    assert_eq!(
        st.pinned_modes,
        [
            HubModePin {
                role: "viewer".into(),
                mode: "simple".into()
            },
            HubModePin {
                role: "control".into(),
                mode: String::new()
            },
            HubModePin {
                role: "admin".into(),
                mode: "installer".into()
            },
        ]
    );
    // A Control client sees its own role's pin only, and no document when
    // the hub has no profile.
    let st = state_from(
        &json!({"serial": "S1", "profile": null}),
        &json!({"role": "control", "mode": "simple"}),
        "control".into(),
    );
    assert!(!st.can_edit && st.profile_document.is_empty());
    assert_eq!(
        st.pinned_modes,
        [HubModePin {
            role: "control".into(),
            mode: "simple".into()
        }]
    );
}

struct Heard(Mutex<Vec<HubAdminEvent>>);

impl HubAdminListener for Heard {
    fn on_hub_admin_event(&self, event: HubAdminEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn wp_hubadmin_events_reach_the_listener_and_the_queue_keeps_the_last() {
    let core = DspiCore::new();
    let heard = Arc::new(Heard(Mutex::new(Vec::new())));
    core.set_hub_admin_listener(Some(heard.clone()));
    // mode.changed is hub-wide: it is told with no device open.
    #[allow(clippy::arithmetic_side_effects)]
    for i in 0..(QUEUE_MAX + 3) {
        let ev = HubAdminEvent::from_hub(
            "mode.changed",
            &json!({"role": "control", "mode": format!("m{i}")}),
        )
        .unwrap();
        core.hub_admin_event(ev);
    }
    assert_eq!(heard.0.lock().unwrap().len(), QUEUE_MAX + 3);
    let q = core.hub_admin_events();
    assert_eq!(q.len(), QUEUE_MAX);
    assert_eq!(q.first().unwrap().mode, "m3", "the oldest went");
    assert!(core.hub_admin_events().is_empty(), "drained");
    // Another device's limits are not this console's business.
    #[cfg(feature = "link")]
    {
        core.hub_admin_event(
            HubAdminEvent::from_hub("limits.changed", &json!({"serial": "X"})).unwrap(),
        );
        assert!(core.hub_admin_events().is_empty());
    }
    core.set_hub_admin_listener(None);
}
