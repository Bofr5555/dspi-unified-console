//! WP P3WGR (review p3c N1): the client read-only mode lets a bulk apply's run
//! be settled. `group.apply.confirm` and `group.apply.cancel` keep or revert
//! what the run already wrote, so they pass as the reads do (Android's
//! `WriteGuardedCore` passes `hubGroupApplyConfirm`/`Cancel` the same way);
//! `group.apply` itself and every other fleet write stay refused.

use crate::hub_server::{HUB_SERVER_READS, HUB_SERVER_SETTLES};
use crate::{DspiCore, DspiError};

#[test]
fn wp_p3wgr_read_only_lets_a_run_be_settled_and_nothing_else() {
    let core = DspiCore::new();
    core.set_client_read_only(true).unwrap();
    let mut inner = core.lock();
    for t in HUB_SERVER_SETTLES {
        assert!(!HUB_SERVER_READS.contains(&t), "{t} is not a read");
        assert_eq!(super::check_hub_request(&mut inner, t), Ok(()), "{t}");
    }
    for t in [
        "group.apply",
        "group.set",
        "group.delete",
        "group.param.set",
        "group.preset.load",
        "fw.image.put",
        "fw.rollout.start",
        "apply.confirm",
    ] {
        assert_eq!(
            super::check_hub_request(&mut inner, t),
            Err(DspiError::ReadOnly),
            "{t}"
        );
    }
}

#[cfg(feature = "link")]
mod through_the_fake_hub {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use dspi_link::fakehub::{FakeHub, HubConfig, JsonResponder};
    use serde_json::{Value, json};

    use crate::hub_server::CAP_GROUP_APPLY;
    use crate::{DspiCore, DspiError, HubFleetTarget};

    fn answers() -> JsonResponder {
        Arc::new(|r: &Value| {
            let t = r["t"].as_str()?;
            Some(match t {
                "group.apply" | "group.apply.confirm" | "group.apply.cancel" => {
                    json!({"run": "a1", "results": [{"serial": "A", "ok": true}]})
                }
                _ => return None,
            })
        })
    }

    #[test]
    fn wp_p3wgr_a_read_only_console_confirms_and_cancels_a_run_but_starts_none() {
        let mut cfg = HubConfig::default();
        cfg.caps.push(CAP_GROUP_APPLY.to_owned());
        let hub = FakeHub::start(cfg).unwrap();
        hub.set_json_responder(Some(answers()));
        let core = DspiCore::new();
        core.link_connect(
            hub.url(),
            "wp-p3wgr".into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let start = Instant::now();
        while core.link_devices().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(10), "no device list");
            std::thread::sleep(Duration::from_millis(5));
        }
        // A run begun before the mode went on.
        let target = HubFleetTarget {
            group: String::new(),
            serials: vec!["A".into()],
        };
        core.hub_group_apply(
            target.clone(),
            "s1".into(),
            String::new(),
            String::new(),
            true,
            60_000,
        )
        .unwrap();
        core.set_client_read_only(true).unwrap();
        assert_eq!(core.hub_group_apply_confirm("a1".into()).unwrap().run, "a1");
        assert_eq!(core.hub_group_apply_cancel("a1".into()).unwrap().run, "a1");
        assert_eq!(
            core.hub_group_apply(
                target,
                "s1".into(),
                String::new(),
                String::new(),
                true,
                60_000
            )
            .err(),
            Some(DspiError::ReadOnly)
        );
        let types: Vec<String> = hub
            .json_requests()
            .into_iter()
            .filter_map(|r| r["t"].as_str().map(str::to_owned))
            .filter(|t| t.starts_with("group."))
            .collect();
        assert_eq!(
            types,
            ["group.apply", "group.apply.confirm", "group.apply.cancel"],
            "the second group.apply is not sent"
        );
        core.link_close();
    }
}
