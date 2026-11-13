//! The shared scenario list (README.md), run by our client. The TypeScript and Swift
//! runs check the same things under the same ids.

use std::time::Duration;

use dspi_link::messages::PollSpec;
use dspi_link::{AuthState, Dir, LinkClient, LinkError, PinSource, status};
use dspi_proto::generated::opcodes as op;

use crate::support::*;

fn paired(b: &Bench, name: &str, role: &str) -> (LinkClient, String) {
    let (c, _) = connect(&b.url(), name, None).unwrap();
    // -- WP HWLK1F -- admin only through a window opened for it
    let pin = if role == "admin" {
        b.admin_pin()
    } else {
        b.pin()
    };
    let token = c.pair(&pin, role).unwrap();
    (c, token)
}

fn set_volume(c: &LinkClient, db: f32) -> u8 {
    c.command(
        0,
        Dir::Set,
        op::REQ_SET_MASTER_VOLUME,
        0,
        2,
        4,
        &db.to_le_bytes(),
    )
    .status
}

#[test]
fn wp_hb7_c1_hello() {
    let b = Bench::pin_hub("c1");
    let (c, state) = connect(&b.url(), "c1", None).unwrap();
    assert_eq!(
        state,
        AuthState::NeedsPairing {
            pin_source: "window".into()
        }
    );
    let hello = c.hello();
    assert_eq!(hello.proto.major, 1);
    assert_eq!(hello.hub.id, b.hub.hub_id());
    assert_eq!(hello.hub.name, HUB_NAME);
    assert_eq!(hello.auth, "pin");
    assert!(hello.has_cap("cmd"));
    assert!(!hello.has_cap("fw_install"));
    assert!(hello.limits.max_frame > 0);
}

#[test]
fn wp_hb7_c2_a_wrong_pin_is_typed_and_a_right_one_then_works() {
    let b = Bench::pin_hub("c2");
    let pin = b.pin();
    let wrong = if pin == "000000" { "111111" } else { "000000" };
    let (c, _) = connect(&b.url(), "c2", None).unwrap();
    assert_eq!(
        c.pair(wrong, "control").unwrap_err(),
        LinkError::WrongPin {
            pin_source: PinSource::Window
        }
    );
    let (c, _) = connect(&b.url(), "c2", None).unwrap();
    c.pair(&pin, "control").unwrap();
    assert!(c.is_ready());
}

#[test]
fn wp_hb7_c3_pair_lists_the_device() {
    let b = Bench::pin_hub("c3");
    let (c, token) = paired(&b, "c3", "admin");
    assert!(!token.is_empty());
    assert_eq!(c.role(), "admin");
    let devices = c.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].handle, 0);
    assert_eq!(devices[0].state, "online");
    assert!(!devices[0].serial.is_empty());
}

#[test]
fn wp_hb7_c4_the_token_reconnects_without_a_pin() {
    let b = Bench::pin_hub("c4");
    let (first, token) = paired(&b, "c4", "control");
    let (second, state) = connect(&b.url(), "c4", Some(token)).unwrap();
    assert_eq!(state, AuthState::Ready);
    assert_ne!(second.session_id(), first.session_id());
    assert_eq!(second.role(), "control");
}

#[test]
fn wp_hb7_c5_commands() {
    let b = Bench::pin_hub("c5");
    let (c, _) = paired(&b, "c5", "control");
    let platform = c.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(platform.status, status::OK);
    assert_eq!(platform.payload.len(), 7);
    let missing = c.command(9, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(missing.status, status::NO_DEVICE);
}

#[test]
fn wp_hb7_c6_a_write_is_heard_with_its_origin() {
    let b = Bench::pin_hub("c6");
    let (first, token) = paired(&b, "c6", "control");
    let (second, _) = connect(&b.url(), "c6", Some(token)).unwrap();
    let own = first.notifications(0);
    let other = second.notifications(0);
    assert_eq!(set_volume(&first, -12.0), status::OK);
    let echo = own.recv_timeout(WAIT).unwrap();
    assert_eq!(Some(echo.origin), first.session_id(), "the echo rule");
    let heard = other.recv_timeout(WAIT).unwrap();
    assert_eq!(Some(heard.origin), first.session_id());
    assert_eq!(heard.packet[0], 2, "a v2 packet");
}

#[test]
fn wp_hb7_c7_a_change_at_the_device_has_origin_0() {
    let b = Bench::pin_hub("c7");
    let (c, _) = paired(&b, "c7", "viewer");
    let rx = c.notifications(0);
    b.knob();
    let heard = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(heard.origin, 0);
}

#[test]
fn wp_hb7_c8_snapshot() {
    let b = Bench::pin_hub("c8");
    let (c, _) = paired(&b, "c8", "viewer");
    let snap = c.snapshot(0).unwrap();
    assert_eq!(snap.handle, 0);
    assert_eq!(snap.bulk().unwrap(), b.remote.image());
}

#[test]
fn wp_hb7_c9_polls_arrive_and_stop() {
    let b = Bench::pin_hub("c9");
    let (c, _) = paired(&b, "c9", "viewer");
    let rx = c.polls(0);
    let spec = PollSpec {
        slot: 1,
        req: op::REQ_GET_STATUS,
        val: 9,
        idx: dspi_proto::VENDOR_INTERFACE,
        len: 41,
        hz: 10,
    };
    let grants = c.poll_subscribe(0, &[spec]).unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].slot, 1);
    for _ in 0..3 {
        let p = rx.recv_timeout(WAIT).unwrap();
        assert_eq!((p.handle, p.slot), (0, 1));
        assert!(!p.payload.is_empty());
    }
    c.poll_unsubscribe(0, &[1]);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "no polls after the unsubscribe"
    );
}

#[test]
fn wp_hb7_c10_a_lock_holds_others_off() {
    let b = Bench::pin_hub("c10");
    let (a, token) = paired(&b, "c10", "control");
    let (other, _) = connect(&b.url(), "c10", Some(token)).unwrap();
    a.lock_acquire(0, "conformance", 5000).unwrap();
    assert_eq!(set_volume(&other, -20.0), status::LOCKED);
    assert_eq!(set_volume(&a, -20.0), status::OK, "the holder writes");
    a.lock_release(0);
    wait_until("the release", || set_volume(&other, -21.0) == status::OK);
}

#[test]
fn wp_hb7_c11_a_viewer_reads_and_may_not_write() {
    let b = Bench::pin_hub("c11");
    let (c, _) = paired(&b, "c11", "viewer");
    assert_eq!(c.role(), "viewer");
    let read = c.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[]);
    assert_eq!(read.status, status::OK);
    assert_eq!(set_volume(&c, -6.0), status::DENIED);
}

#[test]
fn wp_hb7_c12_a_revoke_closes_4002_and_the_token_is_refused() {
    let b = Bench::pin_hub("c12");
    let (c, token) = paired(&b, "c12 revoked", "viewer");
    assert!(b.net.revoke(b.client_id("c12 revoked").unwrap()));
    wait_until("the close", || c.is_closed());
    assert_eq!(c.closed_reason().unwrap().0, Some(4002));
    assert!(!dspi_link::should_reconnect(Some(4002)));
    let (_, state) = connect(&b.url(), "c12 revoked", Some(token)).unwrap();
    assert_eq!(
        state,
        AuthState::TokenRefused {
            pin_source: "window".into()
        }
    );
}

#[test]
fn wp_hb7_c13_a_stop_closes_4003() {
    let b = Bench::pin_hub("c13");
    let (c, _) = paired(&b, "c13", "viewer");
    b.hub.stop();
    wait_until("the close", || c.is_closed());
    assert_eq!(c.closed_reason().unwrap().0, Some(4003));
    assert!(dspi_link::should_reconnect(Some(4003)));
}
