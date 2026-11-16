//! WP CG2: the core's browse methods, on a fake advertiser and the fake hub.

use std::net::{IpAddr, SocketAddr};
use std::sync::Weak;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use dspi_link::discovery::{FakeAdvertiser, HubAdvert};
use dspi_link::fakehub::{Auth, FakeHub, HubConfig};

use super::*;
use crate::LinkPhase;
use crate::link::Tuning;

const WAIT: Duration = Duration::from_secs(8);

fn advert(name: &str, id: &str, port: u16) -> HubAdvert {
    HubAdvert {
        name: name.into(),
        port,
        hub_id: id.into(),
        kind: "console".into(),
        auth: "none".into(),
        device_count: 1,
        serials: vec!["S1".into()],
        web: false,
        path: None,
        tls: false, // -- WP SECDISC --
    }
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < end, "timed out waiting: {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn ip(a: SocketAddr) -> IpAddr {
    a.ip()
}

#[derive(Default)]
struct Count(AtomicUsize);

impl BrowseListener for Count {
    fn on_hubs_changed(&self, _: BrowseSnapshot) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn wp_cg2_start_results_listener_resolve_and_stop() {
    let core = DspiCore::new();
    assert_eq!(core.link_browse_snapshot(), BrowseSnapshot::default());
    assert_eq!(core.link_browse_resolve("x".into()), None);
    assert!(!core.link_browse_reresolve("x".into()));
    core.link_browse_verify("x".into());
    core.link_browse_stop();

    let net = FakeAdvertiser::new();
    let heard = Arc::new(Count::default());
    let listener = Arc::clone(&heard) as Arc<dyn BrowseListener>;
    core.link_browse_start_with(Arc::new(net.clone()), Some(listener), "OWN".into())
        .unwrap();
    until("browsing", || core.link_browse_snapshot().browsing);

    let addr = "10.0.0.1".parse().unwrap();
    net.announce(&advert("Mine", "own", 1), &[addr]);
    net.announce(&advert("Hub", "HID-1", 11915), &[addr]);
    until("found", || core.link_browse_snapshot().hubs.len() == 1);
    let hub = core.link_browse_resolve("hid-1".into()).unwrap();
    assert_eq!(hub.url, "ws://10.0.0.1:11915/dspi/v1");
    assert_eq!(hub.name, "Hub");
    assert_eq!(core.link_browse_resolve("own".into()), None);
    until("heard", || heard.0.load(Ordering::SeqCst) > 0);

    core.link_browse_verify("hid-1".into());
    until("verify", || net.verified().len() == 1);

    core.link_browse_stop();
    assert_eq!(core.link_browse_snapshot(), BrowseSnapshot::default());
    assert_eq!(net.open_now(), 0);
    assert_eq!(core.link_browse_resolve("hid-1".into()), None);
}

#[test]
fn wp_cg2_a_new_start_replaces_the_browse() {
    let core = DspiCore::new();
    let (a, b) = (FakeAdvertiser::new(), FakeAdvertiser::new());
    core.link_browse_start_with(Arc::new(a.clone()), None, String::new())
        .unwrap();
    until("first open", || a.open_now() == 1);
    core.link_browse_start_with(Arc::new(b.clone()), None, String::new())
        .unwrap();
    assert_eq!(a.open_now(), 0, "the old browse has stopped");
    until("second open", || b.open_now() == 1);
}

#[test]
fn wp_cg2_the_listener_can_stop_the_browse_and_the_last_core_can_go_with_it() {
    struct Stopper(Weak<DspiCore>, AtomicUsize);
    impl BrowseListener for Stopper {
        fn on_hubs_changed(&self, s: BrowseSnapshot) {
            if !s.hubs.is_empty()
                && let Some(core) = self.0.upgrade()
            {
                core.link_browse_stop();
                self.1.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let core = DspiCore::new();
    let net = FakeAdvertiser::new();
    let stopper = Arc::new(Stopper(Arc::downgrade(&core), AtomicUsize::new(0)));
    let listener = Arc::clone(&stopper) as Arc<dyn BrowseListener>;
    core.link_browse_start_with(Arc::new(net.clone()), Some(listener), String::new())
        .unwrap();
    net.announce(&advert("Hub", "h", 1), &["10.0.0.1".parse().unwrap()]);
    until("stopped from inside", || {
        stopper.1.load(Ordering::SeqCst) == 1
    });
    until("closed", || net.open_now() == 0);
    assert_eq!(core.link_browse_snapshot(), BrowseSnapshot::default());
}

#[test]
fn wp_cg2_freeing_the_core_stops_the_browse() {
    let core = DspiCore::new();
    let net = FakeAdvertiser::new();
    core.link_browse_start_with(Arc::new(net.clone()), None, String::new())
        .unwrap();
    until("open", || net.open_now() == 1);
    drop(core);
    assert_eq!(net.open_now(), 0);
}

/// F5, end to end: a hub takes a new address under a session. The session is
/// reconnecting; the browser hears the same `hid` from elsewhere; the session
/// follows, and is back, on its own token and without a person.
#[test]
fn wp_cg2_a_session_follows_its_hub_to_a_new_address() {
    let hub_id = "cafe0000-0000-4000-8000-00000000f5f5";
    let cfg = || HubConfig {
        hub_id: hub_id.into(),
        auth: Auth::None,
        ..HubConfig::default()
    };
    let first = FakeHub::start(cfg()).unwrap();
    let core = DspiCore::new();
    let tuning = Tuning {
        backoff_divisor: 50,
        ..Tuning::default()
    };
    let hello = core
        .link_connect_with(
            first.url(),
            "t".into(),
            "1".into(),
            String::new(),
            hub_id.into(),
            tuning,
        )
        .unwrap();
    assert_eq!(hello.hub_id, hub_id);

    let net = FakeAdvertiser::new();
    core.link_browse_start_with(Arc::new(net.clone()), None, String::new())
        .unwrap();
    net.announce(
        &advert("Hub", hub_id, first.addr().port()),
        &[ip(first.addr())],
    );
    until("found", || {
        core.link_browse_resolve(hub_id.into()).is_some()
    });

    // The hub is gone from its address, and nothing answers there.
    first.stop();
    until("reconnecting", || {
        core.link_status().phase == LinkPhase::Reconnecting
    });

    // It is back at another port, and says so.
    let second = FakeHub::start(cfg()).unwrap();
    assert_ne!(second.addr().port(), first.addr().port());
    net.announce(
        &advert("Hub", hub_id, second.addr().port()),
        &[ip(second.addr())],
    );
    until("moved", || {
        core.link_browse_resolve(hub_id.into())
            .is_some_and(|h| h.port == second.addr().port())
    });
    assert!(core.link_browse_reresolve(hub_id.into()));
    until("back on the new address", || {
        core.link_status().ready && !second.sessions().is_empty()
    });
    assert_ne!(core.link_status().phase, LinkPhase::Reconnecting);
    core.link_close();
    second.stop();
}

/// A hub that is not found changes nothing: the session keeps its address.
#[test]
fn wp_cg2_reresolve_of_an_unknown_hub_leaves_the_session_alone() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::default()
    })
    .unwrap();
    let core = DspiCore::new();
    core.link_connect_with(
        hub.url(),
        "t".into(),
        "1".into(),
        String::new(),
        String::new(),
        Tuning::default(),
    )
    .unwrap();
    let net = FakeAdvertiser::new();
    core.link_browse_start_with(Arc::new(net), None, String::new())
        .unwrap();
    assert!(!core.link_browse_reresolve("no-such-hub".into()));
    assert!(core.link_status().ready);
    core.link_close();
    hub.stop();
}
