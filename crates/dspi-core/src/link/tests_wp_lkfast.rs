//! WP LKFAST: fewer round trips before Home over DSPi Link, on the fake bridge
//! hub: the meter poll and the snapshot go out with the bind's identity read,
//! and `device_protocol` takes the board from that read.

use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
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
