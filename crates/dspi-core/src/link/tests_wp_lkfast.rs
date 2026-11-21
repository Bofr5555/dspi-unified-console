//! WP LKFAST: fewer round trips before Home over DSPi Link, on the fake bridge
//! hub: the meter poll and the snapshot go out with the bind's identity read,
//! and `device_protocol` takes the board from that read. LKFAST2: a hub met
//! without TLS is dialled without a TLS try (never a pinned one), and the hub
//! bound last is offered at start.

use std::path::PathBuf;

use dspi_link::credential::Credential;
use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};
use dspi_proto::generated::opcodes as op;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn bridge() -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap()
}

fn connect(core: &DspiCore, hub: &FakeHub) {
    core.link_connect(
        hub.url(),
        "wp-lkfast".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
}

/// JSON requests of type `t` the hub has had.
fn asked(hub: &FakeHub, t: &str) -> usize {
    hub.json_requests().iter().filter(|v| v["t"] == t).count()
}

/// Commands of `opcode` the hub has run.
fn ran(hub: &FakeHub, opcode: u8) -> usize {
    hub.log().iter().filter(|e| e.2 == opcode).count()
}

#[test]
fn wp_lkfast_a_listed_legacy_device_gets_its_meters_and_snapshot_with_the_identity_read() {
    let hub = bridge();
    let core = Arc::new(DspiCore::new());
    connect(&core, &hub);
    let d = core.link_devices().remove(0);
    assert_eq!(d.fw, "1.1.6", "the fake hub lists a legacy device");
    // No answer comes back until released: what goes out meanwhile went out
    // without waiting for the identity read
    hub.hold_replies();
    let bind = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || core.link_select_device(d.handle, d.serial))
    };
    wait_until(
        "the poll, the snapshot and the identity read, all unanswered",
        || {
            asked(&hub, "poll.subscribe") == 1
                && asked(&hub, "device.snapshot") == 1
                && ran(&hub, op::REQ_GET_PLATFORM) == 1
        },
    );
    hub.release_replies();
    bind.join().unwrap().unwrap();
    assert_eq!(ran(&hub, op::REQ_GET_PLATFORM), 1, "one identity read");
    wait_until("a live meter", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    core.link_close();
}

#[test]
fn wp_lkfast_device_protocol_after_a_bind_sends_nothing() {
    let hub = bridge();
    let core = DspiCore::new();
    connect(&core, &hub);
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    let before = hub.log().len();
    let p = core.device_protocol().unwrap();
    assert!(!p.board_name.is_empty());
    let again = core.device_protocol().unwrap();
    assert_eq!(p.board_id, again.board_id);
    let sent: Vec<u8> = hub.log().iter().skip(before).map(|e| e.2).collect();
    assert!(
        !sent.contains(&op::REQ_GET_PLATFORM),
        "the board came with the bind's own platform read: {sent:02X?}"
    );
    core.link_close();
}

// ── LKFAST2: the hub memory ──────────────────────────────────────────────

const PIN: &str = "123456";

/// A fresh folder beside the test binary (never tmpfs).
fn memory_dir(name: &str) -> String {
    let d: PathBuf = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("wp-lkfast-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.to_string_lossy().into_owned()
}

fn core_in(dir: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_set_dir(dir.into()).unwrap();
    core
}

fn dial(core: &DspiCore, hub: &FakeHub, credential: &str, id: &str) -> Result<LinkHello> {
    core.link_connect(
        hub.url(),
        "wp-lkfast".into(),
        "test".into(),
        credential.into(),
        id.into(),
    )
}

#[test]
fn wp_lkfast_a_hub_met_without_tls_skips_the_tls_try_and_rechecks_every_tenth_connect() {
    let dir = memory_dir("count");
    let hub = FakeHub::start(HubConfig::default()).unwrap(); // no TLS: refuses a ClientHello
    let core = core_in(&dir);
    let id = dial(&core, &hub, "", "").unwrap().hub_id;
    core.link_close();
    assert_eq!(hub.tls_refused(), 1, "first contact tries TLS");
    for n in 2..=10 {
        dial(&core, &hub, "", &id).unwrap();
        core.link_close();
        assert_eq!(hub.tls_refused(), 1, "connect {n}: no TLS try");
    }
    // A new core with the same folder (the app restarted) remembers it too
    let core = core_in(&dir);
    dial(&core, &hub, "", &id).unwrap();
    core.link_close();
    assert_eq!(hub.tls_refused(), 2, "the 11th connect tries TLS again");
    dial(&core, &hub, "", &id).unwrap();
    core.link_close();
    assert_eq!(hub.tls_refused(), 2, "and the count starts over");
    assert!(!core.link_security().encrypted);
}

#[test]
fn wp_lkfast_without_a_folder_or_a_hub_id_tls_is_always_tried() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let core = DspiCore::new(); // no folder
    let id = dial(&core, &hub, "", "").unwrap().hub_id;
    core.link_close();
    dial(&core, &hub, "", &id).unwrap();
    core.link_close();
    assert_eq!(hub.tls_refused(), 2, "nothing remembered without a folder");
    let core = core_in(&memory_dir("noid"));
    for _ in 0..2 {
        dial(&core, &hub, "", "").unwrap();
        core.link_close();
    }
    assert_eq!(hub.tls_refused(), 4, "nor for a hub dialled without its id");
}

#[test]
fn wp_lkfast_a_pinned_hub_is_never_dialled_in_plaintext_whatever_the_memory_says() {
    let key = FakeTlsKey::generate();
    let hub = FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        ..HubConfig::default()
    })
    .unwrap();
    hub.set_tls(Some(&key));
    let dir = memory_dir("pinned");
    let core = core_in(&dir);
    let hello = dial(&core, &hub, "", "").unwrap();
    let credential = core.link_pair(PIN.into()).unwrap();
    core.link_close();
    assert!(Credential::parse(&credential).pin.is_some());
    // The memory claims plaintext for this hub (as a spoofer of its id could make it)
    for _ in 0..3 {
        core.hub_memory.met_plaintext(&hello.hub_id, true);
    }
    assert!(core.hub_memory.skip_tls(&hello.hub_id));
    let before = hub.upgrades();
    dial(&core, &hub, &credential, &hello.hub_id).unwrap();
    assert!(core.link_security().encrypted);
    core.link_close();
    assert_eq!(hub.upgrades().0, before.0, "never plaintext");
    assert!(
        !core.hub_memory.skip_tls(&hello.hub_id),
        "met encrypted: the note goes"
    );
    // TLS off at the hub now: still TLS only, refused, nothing in plaintext
    for _ in 0..3 {
        core.hub_memory.met_plaintext(&hello.hub_id, true);
    }
    hub.set_tls(None);
    assert!(dial(&core, &hub, &credential, &hello.hub_id).is_err());
    core.link_close();
    assert_eq!(hub.upgrades().0, before.0, "still never plaintext");
}

#[test]
fn wp_lkfast_an_advert_of_tls_forces_the_tls_try() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let core = core_in(&memory_dir("advert"));
    let id = dial(&core, &hub, "", "").unwrap().hub_id;
    core.link_close();
    assert!(core.hub_memory.skip_tls(&id));
    // The advert says TLS: required (ratchet), so a plaintext-only hub is refused
    let r = core.link_connect_advertised(
        hub.url(),
        "wp-lkfast".into(),
        "test".into(),
        String::new(),
        id.clone(),
        true,
    );
    assert!(r.is_err(), "{r:?}");
    core.link_close();
    assert_eq!(hub.tls_refused(), 2, "TLS was tried");
    assert!(
        !core.hub_memory.skip_tls(&id),
        "and the next unadvertised connect tries it too"
    );
    dial(&core, &hub, "", &id).unwrap();
    core.link_close();
    assert_eq!(hub.tls_refused(), 3);
}

#[test]
fn wp_lkfast_pair_again_rechecks_tls() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let core = core_in(&memory_dir("recheck"));
    let id = dial(&core, &hub, "", "").unwrap().hub_id;
    core.link_close();
    core.link_recheck_tls(id.to_uppercase());
    dial(&core, &hub, "", &id).unwrap();
    core.link_close();
    assert_eq!(hub.tls_refused(), 2);
}

#[test]
fn wp_lkfast_the_hub_bound_last_is_offered_at_start_unless_left_or_usb() {
    let hub = bridge();
    let dir = memory_dir("last");
    let core = core_in(&dir);
    assert_eq!(core.link_start_hub(false), None);
    let hello = dial(&core, &hub, "", "").unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    assert_eq!(
        core.link_start_hub(false),
        None,
        "only once a device is bound"
    );
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.link_close();
    let last = core_in(&dir).link_start_hub(false).expect("the last hub");
    assert_eq!(last.hub_id, hello.hub_id);
    assert_eq!(last.kind, "bridge");
    assert_eq!(last.auth, "none");
    assert_eq!(
        link_ws_url(last.host.clone(), last.port, last.path.clone()),
        hub.url()
    );
    assert!(!last.tls_advertised);
    assert_eq!(core.link_start_hub(true), None, "USB wins");
    core.link_forget_last_hub();
    assert_eq!(
        core_in(&dir).link_start_hub(false),
        None,
        "the user left it"
    );
    dial(&core, &hub, "", &hello.hub_id).unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.link_close();
    assert!(core.link_start_hub(false).is_some(), "bound again");
}
