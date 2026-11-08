//! WP WB4b: a caller-driven Link session (the web's: no supervisor, reader or
//! keepalive thread), turned by `link_tick` against the fake hub. The same
//! supervisor and rules as with threads: it reconnects by itself, re-binds,
//! never sends a write made while the link was down, and ends on 4000.

use dspi_link::client::set_caller_driven_on_this_thread;
use dspi_link::fakehub::{Auth, FakeHub, HubConfig, req};

use super::*;

const WAIT: Duration = Duration::from_secs(8);

fn connect(core: &DspiCore, hub: &FakeHub, token: &str) -> Result<LinkHello> {
    set_caller_driven_on_this_thread(true);
    let r = core.link_connect_with(
        hub.url(),
        "wp-wb4b".into(),
        "test".into(),
        token.into(),
        String::new(),
        Tuning {
            backoff_divisor: 50,
            ..Tuning::default()
        },
    );
    set_caller_driven_on_this_thread(false);
    r
}

/// Turn the session (as the page's timer does) until `phase`; every phase seen on the way.
fn tick_until(core: &DspiCore, phase: LinkPhase) -> Vec<LinkPhase> {
    let start = Instant::now();
    let mut seen = vec![core.link_status().phase];
    while core.link_status().phase != phase {
        assert!(start.elapsed() < WAIT, "no {phase:?}; seen {seen:?}");
        let wait = core.link_tick().expect("a caller-driven session");
        assert!(
            wait <= Duration::from_millis(250),
            "never longer than the keepalive tick: {wait:?}"
        );
        let p = core.link_status().phase;
        if seen.last() != Some(&p) {
            seen.push(p);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    seen
}

fn sets(hub: &FakeHub) -> usize {
    hub.log()
        .iter()
        .filter(|e| e.2 == req::SET_MASTER_VOLUME)
        .count()
}

#[test]
fn wp_wb4b_a_the_caller_turns_the_supervisor_through_a_drop_and_a_reconnect() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let core = DspiCore::new();
    let hello = connect(&core, &hub, "").unwrap();
    assert!(!hello.needs_pairing);
    tick_until(&core, LinkPhase::ChooseDevice);
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial.clone()).unwrap();
    core.fetch_all().unwrap();
    tick_until(&core, LinkPhase::Bound);

    core.set_master_volume(-9.0).unwrap();
    assert_eq!(core.get_state().master_volume_db, -9.0);
    let written = sets(&hub);

    // The link drops: the page sees reconnecting, a write now fails at once and
    // is never sent later, and the core binds the device again by itself.
    hub.drop_connections();
    tick_until(&core, LinkPhase::Reconnecting);
    assert!(matches!(
        core.set_master_volume(-20.0),
        Err(DspiError::Disconnected)
    ));
    let seen = tick_until(&core, LinkPhase::Bound);
    assert!(!seen.contains(&LinkPhase::Closed), "{seen:?}");
    assert_eq!(
        sets(&hub),
        written,
        "the write made while down was never replayed"
    );
    core.set_master_volume(-6.0).unwrap();
    assert_eq!(core.get_state().master_volume_db, -6.0);

    // 4000: the hub ended it for good.
    hub.close_all(4000);
    tick_until(&core, LinkPhase::Closed);
    core.link_close();
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    assert_eq!(core.link_tick(), None, "nothing left to turn");
}

#[test]
fn wp_wb4b_b_pairing_and_the_token_on_a_caller_driven_session() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        ..HubConfig::default()
    })
    .unwrap();
    let core = DspiCore::new();
    let hello = connect(&core, &hub, "").unwrap();
    assert!(hello.needs_pairing);
    assert_eq!(core.link_status().phase, LinkPhase::NeedsPairing);
    let wrong = core.link_pair("000000".into());
    assert!(
        matches!(wrong, Err(DspiError::Link { ref code, .. }) if code == "wrong_pin"),
        "{wrong:?}"
    );
    let token = core.link_pair("123456".into()).unwrap();
    assert!(!token.is_empty());
    tick_until(&core, LinkPhase::ChooseDevice);
    core.link_close();

    // The token opens the next session without a PIN.
    let core2 = DspiCore::new();
    let hello = connect(&core2, &hub, &token).unwrap();
    assert!(!hello.needs_pairing);
    tick_until(&core2, LinkPhase::ChooseDevice);
    core2.link_close();
}
