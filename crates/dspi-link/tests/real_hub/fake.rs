//! The same scenarios against the fake hub, ten times faster. Hubs are the
//! fake's bridge and window shapes, with each timer scaled like the client's.

use std::sync::Arc;
use std::time::Duration;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig, BULK_V32};
use dspi_link::socket::TcpConnector;
use dspi_link::Dir;

use crate::scenarios as sc;
use crate::target::{run, Bench, Skip, Target, TEMP_CLIENT_NAME};

const SCALE: u32 = 10;
const PIN: &str = "482913";

struct FakeBench(Arc<FakeHub>);

impl Bench for FakeBench {
    fn board(&self, present: bool) {
        self.0.set_board_present(present);
    }
    fn slots_in_use(&self) -> Option<usize> {
        Some(self.0.slots_in_use())
    }
    /// What the harness promises a real hub: GETs only, the image as it was, no
    /// lock left behind.
    fn after_scenario(&self) {
        let read = Dir::Get as u8;
        let writes: Vec<_> = self.0.log().into_iter().filter(|(_, dir, _, _)| *dir != read).collect();
        assert!(writes.is_empty(), "the harness wrote: {writes:?}");
        assert_eq!(self.0.image(), BULK_V32, "the image changed");
        assert_eq!(self.0.lock_holder(), None, "a lock was left behind");
    }
}

fn scaled(cfg: HubConfig) -> HubConfig {
    let s = SCALE;
    HubConfig {
        auth_deadline: cfg.auth_deadline / s,
        ping_interval: cfg.ping_interval / s,
        peer_timeout: cfg.peer_timeout / s,
        rate_limit_window: cfg.rate_limit_window / s,
        rate_limit_lockout: cfg.rate_limit_lockout / s,
        board_removal_hold: cfg.board_removal_hold / s,
        ..cfg
    }
}

/// A target on `hub`, with nothing allowed yet.
fn target_on(hub: &Arc<FakeHub>, pin: Option<&str>) -> Target {
    Target {
        host: hub.addr().ip().to_string(),
        port: hub.addr().port(),
        url: hub.url(),
        scale: SCALE,
        slack: Duration::from_millis(1500),
        token: None,
        pin: pin.map(str::to_owned),
        allow_pair: false,
        allow_lockout: false,
        no_board: false,
        operator: false,
        report: None,
        bench: Box::new(FakeBench(Arc::clone(hub))),
    }
}

/// A bridge the harness has paired with, as after `real_setup_pair`.
fn paired_bridge(cfg: HubConfig) -> (Target, Arc<FakeHub>) {
    let hub = Arc::new(FakeHub::start(scaled(cfg)).unwrap());
    let mut t = target_on(&hub, Some(PIN));
    let (c, _) = t.dial(&t.url, None, None).unwrap();
    t.token = Some(c.pair(PIN, "viewer").unwrap());
    c.close();
    (t, hub)
}

fn bridge() -> (Target, Arc<FakeHub>) {
    paired_bridge(HubConfig::bridge())
}

fn window_hub() -> (Target, Arc<FakeHub>) {
    let cfg = HubConfig { auth: Auth::Window(PIN.into()), ..HubConfig::default() };
    let hub = Arc::new(FakeHub::start(scaled(cfg)).unwrap());
    let t = target_on(&hub, Some(PIN));
    (t, hub)
}

/// The scenario ran to its end, gates and all.
fn ran(outcome: crate::target::Outcome) {
    if let Err(Skip(why)) = outcome {
        panic!("skipped: {why}");
    }
}

#[test]
fn wp_xc3_fake_smoke() {
    let (t, hub) = bridge();
    let seen = hub.tokens_seen().len();
    ran(sc::smoke(&t));
    // The settings guard's two sessions use the token; the scenario's lookup sent none
    assert_eq!(hub.tokens_seen().len(), seen + 2);
}

#[test]
fn wp_xc3_fake_smoke_on_an_open_hub() {
    let hub = Arc::new(FakeHub::start(scaled(HubConfig::default())).unwrap());
    ran(sc::smoke(&target_on(&hub, None)));
}

#[test]
fn wp_xc3_fake_1_idle() {
    ran(sc::idle_session(&bridge().0));
}

#[test]
fn wp_xc3_fake_1_idle_with_no_board() {
    let (mut t, _hub) = paired_bridge(HubConfig { board_attached: false, ..HubConfig::bridge() });
    t.no_board = true;
    ran(sc::idle_session(&t));
}

#[test]
fn wp_xc3_fake_2_wrong_pin_keeps_token() {
    let (mut t, _hub) = bridge();
    t.allow_lockout = true;
    ran(sc::wrong_pin_keeps_token(&t));
}

#[test]
fn wp_xc3_fake_3_dark_5s_polling() {
    ran(sc::dark_network(&bridge().0, 5.0, true));
}

#[test]
fn wp_xc3_fake_3_dark_20s_polling() {
    ran(sc::dark_network(&bridge().0, 20.0, true));
}

#[test]
fn wp_xc3_fake_3_dark_32s_polling() {
    ran(sc::dark_network(&bridge().0, 32.0, true));
}

#[test]
fn wp_xc3_fake_3_dark_60s_polling() {
    ran(sc::dark_network(&bridge().0, 60.0, true));
}

#[test]
fn wp_xc3_fake_3_dark_60s_idle() {
    ran(sc::dark_network(&bridge().0, 60.0, false));
}

#[test]
fn wp_xc3_fake_4_slots_full() {
    let (mut t, _hub) = bridge();
    t.allow_lockout = true;
    ran(sc::slots_full(&t));
}

#[test]
fn wp_xc3_fake_4_no_cap_on_a_console_hub() {
    let cfg = HubConfig { auth: Auth::None, ..HubConfig::default() };
    let hub = Arc::new(FakeHub::start(scaled(cfg)).unwrap());
    let mut t = target_on(&hub, None);
    t.allow_lockout = true;
    ran(sc::slots_full(&t));
}

#[test]
fn wp_xc3_fake_6_no_board_pairing() {
    let cfg = HubConfig { board_attached: false, ..HubConfig::bridge() };
    let hub = Arc::new(FakeHub::start(scaled(cfg)).unwrap());
    let mut t = target_on(&hub, Some(PIN));
    t.allow_pair = true;
    t.no_board = true;
    ran(sc::no_board_pairing(&t));
}

#[test]
fn wp_xc3_fake_6_board_flap() {
    let (mut t, _hub) = bridge();
    t.operator = true;
    ran(sc::board_flap(&t));
}

#[test]
fn wp_xc3_fake_7_attach_gets() {
    ran(sc::attach_gets(&bridge().0));
}

#[test]
fn wp_xc3_fake_8_pair_cut_at_ok_on_each_kind_of_pin() {
    for (mut t, _hub) in [bridge(), window_hub()] {
        t.token = None;
        t.allow_pair = true;
        ran(sc::pair_cut_at_ok(&t));
    }
}

#[test]
fn wp_xc3_fake_8_pair_late_reply_on_each_kind_of_pin() {
    for (mut t, _hub) in [bridge(), window_hub()] {
        t.token = None;
        t.allow_pair = true;
        ran(sc::pair_late_reply(&t));
    }
}

/// Without their variables the gated scenarios send nothing at all: not a
/// connection, not a PIN, not a token.
#[test]
fn wp_xc3_gates_stop_a_scenario_before_it_touches_the_hub() {
    let (t, hub) = bridge();
    let before = (hub.tokens_seen().len(), hub.log().len());
    type Scenario = fn(&Target) -> crate::target::Outcome;
    let gated: [(&str, Scenario); 6] = [
        ("wrong_pin", sc::wrong_pin_keeps_token),
        ("slots_full", sc::slots_full),
        ("pair_cut", sc::pair_cut_at_ok),
        ("pair_late", sc::pair_late_reply),
        ("no_board_pairing", sc::no_board_pairing),
        ("board_flap", sc::board_flap),
    ];
    for (name, scenario) in gated {
        assert!(scenario(&t).is_err(), "{name} ran without its variable");
    }
    assert_eq!((hub.tokens_seen().len(), hub.log().len()), before);
    assert!(crate::target::wait_until(Duration::from_secs(2), || hub.slots_in_use() == 0));
}

/// The fake has no client management, so this is the path a viewer gets on a real
/// hub: nothing is revoked, and the operator is told.
#[test]
fn wp_xc3_forget_pairing_reports_a_refusal() {
    let (t, _hub) = bridge();
    let token = t.token.clone().unwrap();
    let outcome = crate::target::forget_pairing(&TcpConnector::default(), &t.url, &token, &[TEMP_CLIENT_NAME]);
    assert!(matches!(outcome, crate::target::Forgotten::Refused(why) if why.contains("unknown_type")));
}

/// The guard catches what it is there for: settings that changed under a scenario.
#[test]
#[should_panic(expected = "the settings changed")]
fn wp_xc3_the_settings_guard_notices_a_change() {
    let (t, hub) = bridge();
    let _ = run(&t, "changes_a_setting", &[], |_, _| {
        hub.external_change(dspi_link::fakehub::OFF_MASTER_VOLUME, &(-3.0f32).to_le_bytes(), 1, 99);
        Ok(())
    });
}

/// And so does the check behind it, had the harness itself written.
#[test]
#[should_panic(expected = "the image changed")]
fn wp_xc3_the_bench_check_notices_a_change() {
    let (t, hub) = bridge();
    hub.external_change(dspi_link::fakehub::OFF_MASTER_VOLUME, &(-3.0f32).to_le_bytes(), 1, 99);
    t.bench.after_scenario();
}

/// With no PIN known, a "wrong" one could be the hub's, so nothing is tried.
#[test]
fn wp_xc3_wrong_pin_needs_the_pin() {
    let (mut t, hub) = bridge();
    t.allow_lockout = true;
    t.pin = None;
    let seen = hub.tokens_seen().len();
    assert!(sc::wrong_pin_keeps_token(&t).is_err());
    assert_eq!(hub.tokens_seen().len(), seen);
}

/// A hub that refuses client management: teardown skips, keeps the file and says
/// what to remove by hand.
#[test]
fn wp_xc3_teardown_skips_when_the_hub_refuses() {
    let (t, _hub) = bridge();
    let file = std::path::Path::new("/nonexistent/token");
    let Err(Skip(why)) = crate::real::teardown(&t, file) else { panic!("revoked on a hub with no client management") };
    assert!(why.contains("hub's own UI") && why.contains(TEMP_CLIENT_NAME), "{why}");
}
