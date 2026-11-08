//! F7: the first timeout probes (a ping; WP LK2: only silence ends the
//! session); connect is bounded.

use std::net::TcpListener;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dspi_link::fakehub::{req, DeviceListFault, HubConfig};
use dspi_link::socket::{TcpConnector, WsMessage, WsRecv};
use dspi_link::{status, Dir, LinkClient, LinkError, SessionConfig, Timing};

use crate::support::*;

fn short(clock: &std::sync::Arc<dspi_link::clock::ManualClock>) -> SessionConfig {
    let mut cfg = session_on(clock);
    cfg.timing.request_timeout = Duration::from_millis(20);
    cfg
}

fn timed_out_command(c: &LinkClient) {
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT);
}

fn pings(hub: &MockHub) -> u32 {
    hub.pings.load(Ordering::SeqCst)
}

// WP LK2 (LK1-c): an unanswered request no longer ends the session after 5 s;
// only the silence limit does.
#[test]
fn wp_lk2_a_command_timeout_pings_and_only_silence_ends_the_session() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, short(&clock));
    assert_eq!(pings(&hub), 0, "the clock has not moved: no regular ping yet");
    timed_out_command(&c);
    assert!(wait(|| pings(&hub) == 1), "the probe goes out at once, not at the next regular ping");
    clock.advance(Duration::from_secs(6));
    hub.settle();
    assert!(!c.is_closed(), "an unanswered request and a lost ping are not a dead link");
    clock.advance(Timing::default().silence_limit - Duration::from_secs(6) - Duration::from_millis(500));
    hub.settle();
    assert!(!c.is_closed(), "within the silence limit");
    clock.advance(Duration::from_millis(1100));
    assert!(wait(|| c.is_closed()), "dead once the hub was silent for the limit");
    assert_eq!(c.closed_reason().unwrap(), (None, "connection lost".to_owned()));
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::NO_DEVICE);
}

#[test]
fn wp_lk2_the_defaults_ping_every_5_s_and_give_up_after_30_s_of_silence() {
    let t = Timing::default();
    assert_eq!(t.ping_interval, Duration::from_secs(5));
    assert_eq!(t.silence_limit, Duration::from_secs(30));
}

#[test]
fn wp_l1_f7_a_slow_device_on_a_live_hub_is_not_a_dead_session() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, short(&clock));
    hub.answer_pings.store(true, Ordering::SeqCst);
    timed_out_command(&c);
    assert!(wait(|| pings(&hub) == 1));
    hub.settle(); // The pong is read
    clock.advance(Duration::from_secs(6));
    hub.settle();
    assert!(!c.is_closed(), "the ping was answered");
    // And the next timeout probes again
    timed_out_command(&c);
    assert!(wait(|| pings(&hub) >= 2));
}

#[test]
fn wp_l1_f7_any_inbound_frame_answers_the_probe() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, short(&clock));
    timed_out_command(&c);
    assert!(wait(|| pings(&hub) == 1));
    hub.push(WsRecv::Message(WsMessage::Text(r#"{"t":"hub.config.changed"}"#.into())));
    hub.settle();
    clock.advance(Duration::from_secs(6));
    hub.settle();
    assert!(!c.is_closed());
}

#[test]
fn wp_l1_f7_a_control_request_timeout_probes_too() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, short(&clock));
    assert_eq!(c.lock_acquire(0, "x", 1000), Err(LinkError::Timeout));
    assert!(wait(|| pings(&hub) == 1));
}

#[test]
fn wp_l1_f7_half_open_through_real_sockets() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::default());
    let (c, _) =
        LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &short(&clock)).unwrap();
    hub.stall(true);
    timed_out_command(&c);
    std::thread::sleep(Duration::from_millis(20)); // The probe goes out
    clock.advance(Timing::default().silence_limit + Duration::from_millis(100));
    assert!(wait(|| c.is_closed()), "at the next reader tick");
}

#[test]
fn wp_l1_f7_a_failed_send_ends_the_session_for_requests_too() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, short(&clock));
    hub.fail_sends.store(true, Ordering::SeqCst);
    assert!(matches!(c.lock_acquire(0, "x", 1000), Err(LinkError::Io(_))));
    assert!(c.is_closed(), "the framing is unknown after a failed write");
}

#[test]
fn wp_l1_f7_a_handshake_that_never_answers_is_bounded() {
    let quiet = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/dspi/v1", quiet.local_addr().unwrap());
    let connector = TcpConnector { connect_timeout: Duration::from_millis(100) };
    let started = Instant::now();
    let e = LinkClient::connect(&connector, &url, &opts(None), None).err().unwrap();
    assert_eq!(e, LinkError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
}

#[test]
fn wp_l1_f7_connect_has_one_budget_for_all_its_steps() {
    let hub = dspi_link::fakehub::FakeHub::start(HubConfig::default()).unwrap();
    hub.fail_device_list(DeviceListFault::Silent, 1); // The last step
    let cfg = SessionConfig {
        timing: Timing {
            connect_budget: Duration::from_millis(300),
            request_timeout: Duration::from_secs(10),
            ..Timing::default()
        },
        ..SessionConfig::default()
    };
    let started = Instant::now();
    let e = LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &cfg).err().unwrap();
    assert_eq!(e, LinkError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
}
