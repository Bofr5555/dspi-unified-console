#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::clock::TestClock;
use crate::hub::{Hub, HubConfig};

struct Rig {
    hub: Hub,
    clock: Arc<TestClock>,
}

impl Rig {
    fn new() -> Self {
        let clock = Arc::new(TestClock::new());
        let hub = Hub::start(HubConfig {
            services: ServiceConfig {
                clock: Arc::clone(&clock) as Arc<dyn HubClock>,
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("FEATUI hub")
        })
        .unwrap();
        Self { hub, clock }
    }

    fn session(&self, role: Role, name: &str) -> Arc<Session> {
        self.hub.handle().open_principal(role, None, name).unwrap()
    }

    fn ask(&self, s: &Arc<Session>, msg: Value) -> Value {
        serde_json::from_str(&self.hub.handle().service_message(s, &msg).unwrap()).unwrap()
    }

    fn list(&self, s: &Arc<Session>) -> Vec<Value> {
        let r = self.ask(s, json!({"t": "presence.list", "id": 1}));
        assert_eq!(r["t"], "ok", "{r}");
        r["presence"].as_array().unwrap().clone()
    }
}

#[test]
fn wp_featui_presence_is_set_listed_cleaned_and_cleared() {
    let rig = Rig::new();
    let desk = rig.session(Role::Control, "Studio Mac");
    let viewer = rig.session(Role::Viewer, "Phone");
    assert!(
        rig.hub.handle().service_caps().contains(&CAP),
        "the cap is in hello"
    );
    assert!(rig.list(&viewer).is_empty());
    let r = rig.ask(
        &desk,
        json!({"t": "presence.set", "id": 2, "kind": "agent", "label": "Claude\u{202e}", "serial": "DEMO-ALINK-A",
               "what": "Setting a gain\u{200b} on Out 1\n", "ttl_s": 20}),
    );
    assert_eq!(r, json!({"t": "ok", "id": 2, "v": 1}));
    let rows = rig.list(&viewer);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["label"], "Claude", "a bidi override is dropped");
    assert_eq!(rows[0]["what"], "Setting a gain on Out 1");
    assert_eq!(rows[0]["serial"], "DEMO-ALINK-A");
    assert_eq!(
        rows[0]["by"], "Studio Mac",
        "the hub's own label of the session"
    );
    assert_eq!(rows[0]["ttl_s"], 20);
    // Cleared by its session
    assert_eq!(
        rig.ask(&desk, json!({"t": "presence.set", "id": 3, "ttl_s": 0}))["t"],
        "ok"
    );
    assert!(rig.list(&viewer).is_empty());
}

#[test]
fn wp_featui_presence_refusals_and_bounds() {
    let rig = Rig::new();
    let viewer = rig.session(Role::Viewer, "Phone");
    let desk = rig.session(Role::Admin, "Desk");
    let code = |s: &Arc<Session>, m: Value| rig.ask(s, m)["code"].as_str().unwrap_or("").to_owned();
    assert_eq!(
        code(
            &viewer,
            json!({"t": "presence.set", "label": "x", "ttl_s": 10})
        ),
        "denied"
    );
    assert_eq!(
        code(&desk, json!({"t": "presence.set", "label": "x"})),
        "bad_request"
    );
    assert_eq!(
        code(
            &desk,
            json!({"t": "presence.set", "label": "x", "ttl_s": 4})
        ),
        "bad_request"
    );
    assert_eq!(
        code(
            &desk,
            json!({"t": "presence.set", "label": "x", "ttl_s": 61})
        ),
        "bad_request"
    );
    assert_eq!(
        code(
            &desk,
            json!({"t": "presence.set", "label": " \u{202e} ", "ttl_s": 10})
        ),
        "bad_request"
    );
    assert_eq!(
        code(
            &desk,
            json!({"t": "presence.set", "kind": "person", "label": "x", "ttl_s": 10})
        ),
        "bad_request"
    );
    let long = "é".repeat(300);
    assert_eq!(
        rig.ask(
            &desk,
            json!({"t": "presence.set", "label": long, "what": long, "ttl_s": 10})
        )["t"],
        "ok"
    );
    let rows = rig.list(&viewer);
    assert_eq!(
        rows[0]["label"].as_str().unwrap().chars().count(),
        LABEL_MAX
    );
    assert_eq!(rows[0]["what"].as_str().unwrap().chars().count(), WHAT_MAX);
    // Another family's message is not this service's
    assert!(
        rig.hub
            .handle()
            .service_message(&desk, &json!({"t": "presence.other"}))
            .is_none()
    );
}

#[test]
fn wp_featui_presence_ends_at_its_time_and_with_its_session() {
    let rig = Rig::new();
    let viewer = rig.session(Role::Viewer, "Phone");
    let a = rig.session(Role::Control, "A");
    let b = rig.session(Role::Control, "B");
    for s in [&a, &b] {
        assert_eq!(
            rig.ask(
                s,
                json!({"t": "presence.set", "label": "Agent", "ttl_s": 10})
            )["t"],
            "ok"
        );
    }
    assert_eq!(rig.list(&viewer).len(), 2);
    rig.clock.advance(Duration::from_secs(6));
    // Set again: kept 10 s from now
    assert_eq!(
        rig.ask(
            &a,
            json!({"t": "presence.set", "label": "Agent", "ttl_s": 10})
        )["t"],
        "ok"
    );
    rig.clock.advance(Duration::from_secs(5));
    let rows = rig.list(&viewer);
    assert_eq!(rows.len(), 1, "B's time is up: {rows:?}");
    assert_eq!(rows[0]["by"], "A");
    rig.hub.handle().close_session(a.id());
    assert!(rig.list(&viewer).is_empty(), "its session closed");
}
