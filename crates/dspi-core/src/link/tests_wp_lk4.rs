//! WP LK4: the meter feed from select's start, a leaner probe reused on a
//! re-attach and the adaptive meter rate on a bridge, on the fake hub. (The
//! batched control-surface read is tested in dspi-session: the fake hub's
//! device has no control surfaces.)

use std::io;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
use dspi_link::socket::{WsMessage, WsPair, WsReceiver, WsSender};
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

fn bridge_with(max_inflight: u32) -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::None,
        max_inflight,
        ..HubConfig::bridge()
    })
    .unwrap()
}

/// Every tunnelled command (a binary frame) goes out this much later; the
/// JSON control plane (the poll subscription) is not slowed: a link where the
/// probe's reads are slow.
#[derive(Default)]
struct SlowCommands {
    delay_ms: AtomicU64,
}

struct SlowConnector(Arc<SlowCommands>);

impl WsConnector for SlowConnector {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        let tx: Box<dyn WsSender> = Box::new(SlowSender(tx, Arc::clone(&self.0)));
        let rx: Box<dyn WsReceiver> = rx;
        Ok((tx, rx))
    }
}

struct SlowSender(Box<dyn WsSender>, Arc<SlowCommands>);

impl WsSender for SlowSender {
    fn send(&self, msg: WsMessage) -> io::Result<()> {
        let delay = self.1.delay_ms.load(Ordering::SeqCst);
        if delay > 0 && matches!(msg, WsMessage::Binary(_)) {
            std::thread::sleep(Duration::from_millis(delay));
        }
        self.0.send(msg)
    }

    fn ping(&self) -> io::Result<()> {
        self.0.ping()
    }

    fn close(&self, code: u16) {
        self.0.close(code);
    }
}

fn connect_slow(core: &DspiCore, hub: &FakeHub, slow: &Arc<SlowCommands>) {
    core.link_connect_with(
        hub.url(),
        "wp-lk4".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            connector: Arc::new(SlowConnector(Arc::clone(slow))),
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
}

fn select(core: &DspiCore) -> Result<()> {
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial)
}

fn bound_side(core: &DspiCore) -> Arc<LinkSide> {
    let inner = core.lock();
    let side = inner
        .link
        .as_ref()
        .and_then(|s| s.bound.as_ref())
        .and_then(|b| b.side.clone());
    side.expect("a bound side")
}

/// Commands of `opcode` the hub has run.
fn ran(hub: &FakeHub, opcode: u8) -> usize {
    hub.log().iter().filter(|e| e.2 == opcode).count()
}

#[test]
fn wp_lk4_select_binds_on_the_snapshot_and_the_meters_run_before_the_probe_completes() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    // Every command 60 ms late: the probe's forty-odd reads take seconds
    slow.delay_ms.store(60, Ordering::SeqCst);
    let probe_op = op::REQ_GET_LG_SOUND_SYNC_ENABLE;
    select(&core).unwrap();
    // Bound on the identity and the snapshot: no feature read yet, the
    // mirror shows the device's settings, and fetch_all reads nothing more
    assert_eq!(ran(&hub, probe_op), 0, "the select read only what binds");
    // The hub lists the identity: not even GET_PLATFORM before the bind
    assert_eq!(hub.log().len(), 0, "the snapshot alone: {:02X?}", hub.log());
    core.fetch_all().unwrap();
    assert_eq!(
        core.get_state().num_channels,
        core.meter_path().unwrap().num_channels
    );
    let mut live_before_probe = false;
    let start = Instant::now();
    while start.elapsed() < WAIT {
        if core.fetch_status().is_ok_and(|s| !s.stale) {
            live_before_probe = ran(&hub, probe_op) == 0
                || core
                    .lock()
                    .conn
                    .as_ref()
                    .is_some_and(|c| c.pending_probe.is_some());
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(live_before_probe, "a live meter before the probe completed");
    // The probe completes by itself, and says so with a full refresh
    wait_until("the completed probe", || ran(&hub, probe_op) == 1);
    core.ensure_probed(); // waits for the completion in progress
    let caps = core
        .lock()
        .conn()
        .unwrap()
        .session()
        .unwrap()
        .capabilities()
        .clone();
    assert!(!caps.features.is_empty());
    assert_eq!(
        ran(&hub, op::REQ_GET_PLATFORM),
        1,
        "the device's own word, after"
    );
    // Every feature asked and answered (the fake firmware stalls them all)
    assert!(
        caps.features
            .iter()
            .all(|f| f.evidence.ends_with("answered") || f.evidence.ends_with("stalled")),
        "{:?}",
        caps.features
            .iter()
            .map(|f| f.evidence.clone())
            .collect::<Vec<_>>()
    );
    wait_until("the refresh", || {
        core.process_notifications().flags & crate::consts::NOTIFY_REFRESHED != 0
    });
    core.link_close();
}

#[test]
fn wp_lk4_a_call_that_needs_the_features_completes_the_probe_first() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    slow.delay_ms.store(30, Ordering::SeqCst);
    select(&core).unwrap();
    // The device name is a probed feature: asked at once, it is answered from
    // the completed probe (the fake hub's firmware has none: Unsupported)
    let _ = core.cached_device_name();
    let complete = core
        .lock()
        .conn()
        .unwrap()
        .session()
        .unwrap()
        .capabilities()
        .features
        .len();
    assert!(complete > 0, "the probe completed first");
    assert_eq!(
        ran(&hub, op::REQ_GET_LG_SOUND_SYNC_ENABLE),
        1,
        "probed once"
    );
    core.link_close();
}

#[test]
fn wp_lk4_a_failed_select_leaves_no_meter_path() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    // Its reads go unanswered: the select waits
    hub.hold_replies();
    let opener = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || select(&core))
    };
    // The snapshot (and any identity read) goes unanswered meanwhile
    std::thread::sleep(Duration::from_millis(300));
    hub.drop_connections();
    assert!(opener.join().unwrap().is_err(), "the select lost its link");
    assert!(core.meter_path().is_none(), "cleared with the failed open");
    assert!(matches!(core.fetch_status(), Err(DspiError::Disconnected)));
    core.link_close();
}

#[test]
fn wp_lk4_a_re_attach_reuses_the_probe_of_the_same_device() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    select(&core).unwrap();
    core.ensure_probed();
    core.fetch_all().unwrap();
    // One feature probe only the full probe reads
    let probe_op = op::REQ_GET_LG_SOUND_SYNC_ENABLE;
    assert_eq!(ran(&hub, probe_op), 1);
    let generation = core.lock().conn().unwrap().generation;
    let caps = core
        .lock()
        .conn()
        .unwrap()
        .session()
        .unwrap()
        .capabilities()
        .clone();

    hub.drop_connections();
    wait_until("the re-attach", || {
        core.lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.generation != generation)
            && core.link_status().phase == LinkPhase::Bound
    });
    assert_eq!(ran(&hub, probe_op), 1, "no feature reads on the re-attach");
    let again = core
        .lock()
        .conn()
        .unwrap()
        .session()
        .unwrap()
        .capabilities()
        .clone();
    assert_eq!(
        format!("{:?}", again.features),
        format!("{:?}", caps.features)
    );
    assert_eq!(again.max_bands, caps.max_bands);
    wait_until("live meters again", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    core.link_close();
}

#[test]
fn wp_lk4_a_fresh_select_probes_in_full() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    select(&core).unwrap();
    core.ensure_probed();
    core.disconnect();
    // A new select (not a re-attach) never reuses: the user chose again
    select(&core).unwrap();
    core.ensure_probed();
    assert_eq!(ran(&hub, op::REQ_GET_LG_SOUND_SYNC_ENABLE), 2);
    core.link_close();
}

#[test]
fn wp_lk4_a_stalled_bridge_feed_runs_slow_then_recovers() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    select(&core).unwrap();
    let side = bound_side(&core);
    wait_until("live meters", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    assert_eq!(guard(&side.feed).meter_hz, Some(METER_HZ_BRIDGED));

    // A stall: no frame for longer than FEED_LOST. The hub stops sending
    // (as the stalled link would), the frames on their way are drained
    side.client.poll_unsubscribe(side.handle, &[METER_SLOT]);
    std::thread::sleep(Duration::from_millis(300));
    let now = Instant::now();
    {
        let mut feed = guard(&side.feed);
        side.drain(&mut feed, now);
        feed.latest.remove(&METER_SLOT);
        feed.meter_subscribed_at = now.checked_sub(FEED_LOST * 2);
        feed.meter_asked = Some(now);
    }
    side.maintain(now);
    assert_eq!(
        guard(&side.feed).meter_hz,
        Some(METER_HZ_STALLED),
        "asked again slowly"
    );
    assert!(guard(&side.feed).meter_slow_since.is_some());

    // Frames flow again, but not yet for long: stays slow
    wait_until("a slow frame", || {
        let _ = core.fetch_status();
        guard(&side.feed).latest.contains_key(&METER_SLOT)
    });
    guard(&side.feed).meter_asked = Some(Instant::now());
    side.maintain(Instant::now());
    assert_eq!(guard(&side.feed).meter_hz, Some(METER_HZ_STALLED));

    // Flowing for METER_SLOW_FOR since the stall: the full rate again
    {
        let mut feed = guard(&side.feed);
        feed.meter_slow_since = Instant::now().checked_sub(METER_SLOW_FOR);
        feed.meter_asked = Some(Instant::now());
    }
    wait_until("a fresh frame", || {
        let _ = core.fetch_status();
        guard(&side.feed)
            .latest
            .get(&METER_SLOT)
            .is_some_and(|f| f.1.elapsed() < Duration::from_millis(500))
    });
    side.maintain(Instant::now());
    assert_eq!(guard(&side.feed).meter_hz, Some(METER_HZ_BRIDGED));
    assert!(guard(&side.feed).meter_slow_since.is_none());
    core.link_close();
}

#[test]
fn wp_lk4_a_stall_on_another_hub_keeps_the_rate() {
    // Our own hub has no 10 s unwritable rule: a silent feed is asked again as before
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::default()
    })
    .unwrap();
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    select(&core).unwrap();
    let side = bound_side(&core);
    wait_until("live meters", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    let full = guard(&side.feed).meter_hz;
    side.client.poll_unsubscribe(side.handle, &[METER_SLOT]);
    std::thread::sleep(Duration::from_millis(300));
    let now = Instant::now();
    {
        let mut feed = guard(&side.feed);
        side.drain(&mut feed, now);
        feed.latest.remove(&METER_SLOT);
        feed.meter_subscribed_at = now.checked_sub(FEED_LOST * 2);
        feed.meter_asked = Some(now);
    }
    side.maintain(now);
    assert_eq!(guard(&side.feed).meter_hz, full);
    assert!(guard(&side.feed).meter_slow_since.is_none());
    core.link_close();
}

/// The first connection's inbound traffic can be dropped (a session whose
/// hub went quiet, as after the bridge closed it during an outage); later
/// connections are untouched.
#[derive(Default)]
struct Quiet {
    muted: AtomicBool,
    connects: std::sync::atomic::AtomicUsize,
}

struct QuietConnector(Arc<Quiet>);

impl WsConnector for QuietConnector {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        let first = self.0.connects.fetch_add(1, Ordering::SeqCst) == 0;
        let rx: Box<dyn WsReceiver> = Box::new(QuietReceiver(rx, Arc::clone(&self.0), first));
        Ok((tx, rx))
    }
}

struct QuietReceiver(Box<dyn WsReceiver>, Arc<Quiet>, bool);

impl WsReceiver for QuietReceiver {
    fn recv(&mut self) -> dspi_link::socket::WsRecv {
        use dspi_link::socket::WsRecv;
        loop {
            let r = self.0.recv();
            let quiet = self.2 && self.1.muted.load(Ordering::SeqCst);
            match r {
                WsRecv::Message(_) | WsRecv::Alive if quiet => {}
                other => return other,
            }
        }
    }
}

fn connect_quiet(
    core: &DspiCore,
    hub: &FakeHub,
    quiet: &Arc<Quiet>,
    clock: &Arc<dspi_link::clock::ManualClock>,
) {
    core.link_connect_with(
        hub.url(),
        "wp-lk4".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            connector: Arc::new(QuietConnector(Arc::clone(quiet))),
            clock: Arc::clone(clock) as Arc<dyn Clock>,
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
}

#[test]
fn wp_lk4_a_silent_bridge_session_is_replaced_once_a_new_one_connects() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let quiet = Arc::new(Quiet::default());
    let clock = Arc::new(dspi_link::clock::ManualClock::new());
    connect_quiet(&core, &hub, &quiet, &clock);
    select(&core).unwrap();
    core.ensure_probed();
    core.fetch_all().unwrap();
    let generation = core.lock().conn().unwrap().generation;
    let old = core.lock().link.as_ref().unwrap().client.clone().unwrap();

    // Not yet past the bridge's limit: nothing happens
    quiet.muted.store(true, Ordering::SeqCst);
    clock.advance(PROBE_AFTER - Duration::from_secs(2));
    std::thread::sleep(FEED_CHECK * 2);
    assert_eq!(quiet.connects.load(Ordering::SeqCst), 1);

    // Past it: a new session connects, so the old one is dead
    clock.advance(Duration::from_secs(3));
    wait_until("the replacement", || {
        core.lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.generation != generation)
            && core.link_status().phase == LinkPhase::Bound
    });
    assert_eq!(quiet.connects.load(Ordering::SeqCst), 2);
    assert!(old.is_closed(), "the silent session was closed");
    wait_until("live meters again", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    core.link_close();
}

#[test]
fn wp_lk4_a_silent_session_on_another_hub_is_left_to_the_silence_limit() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::default()
    })
    .unwrap();
    let core = DspiCore::new();
    let quiet = Arc::new(Quiet::default());
    let clock = Arc::new(dspi_link::clock::ManualClock::new());
    connect_quiet(&core, &hub, &quiet, &clock);
    select(&core).unwrap();
    quiet.muted.store(true, Ordering::SeqCst);
    clock.advance(PROBE_AFTER + Duration::from_secs(1));
    std::thread::sleep(FEED_CHECK * 3);
    assert_eq!(quiet.connects.load(Ordering::SeqCst), 1, "no probe dial");
    assert_eq!(core.link_status().phase, LinkPhase::Bound);
    core.link_close();
}

#[test]
fn wp_lk4_the_listed_identity_is_the_platform_reply_the_device_gives() {
    let d = DeviceInfo {
        handle: 0,
        serial: "ABC".into(),
        platform: Some(1),
        fw: Some("1.1.6".into()),
        fw_beta: Some(5),
        outputs: Some(9),
        inputs: Some(8),
        wire_version: Some(32),
        state: "online".into(),
        ..DeviceInfo::default()
    };
    let l = listed_identity(&d).unwrap();
    assert_eq!(l.platform, vec![1, 1, 0x16, 9, 1, 6, 5]);
    assert_eq!(l.serial, "ABC");
    // Anything missing or odd: the bind reads the device instead
    for odd in [
        DeviceInfo {
            fw_beta: None,
            ..d.clone()
        },
        DeviceInfo {
            fw: Some("1.1".into()),
            ..d.clone()
        },
        DeviceInfo {
            fw: Some("1.1.6-beta5".into()),
            ..d.clone()
        },
        DeviceInfo {
            platform: None,
            ..d.clone()
        },
        DeviceInfo {
            outputs: Some(300),
            ..d.clone()
        },
        DeviceInfo {
            serial: String::new(),
            ..d.clone()
        },
    ] {
        assert!(listed_identity(&odd).is_none(), "{odd:?}");
    }
}

#[test]
fn wp_lk4_fetch_all_leaves_the_per_item_reads_to_a_completion_in_progress() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    slow.delay_ms.store(40, Ordering::SeqCst);
    select(&core).unwrap();
    // The background completion has started (it reads GET_PLATFORM first)
    wait_until("the completion", || ran(&hub, op::REQ_GET_PLATFORM) == 1);
    let started = Instant::now();
    core.fetch_all().unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "fetch_all read: {:?}",
        started.elapsed()
    );
    core.ensure_probed();
    assert_eq!(
        ran(&hub, op::REQ_PRESET_GET_DIR),
        1,
        "the per-item reads once, by the completion"
    );
    core.link_close();
}

#[test]
fn wp_lk4_a_select_whose_session_is_replaced_binds_on_the_new_one() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    hub.hold_replies();
    let opener = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || select(&core))
    };
    std::thread::sleep(Duration::from_millis(300));
    // The hub drops the session mid-select, then answers again
    hub.drop_connections();
    std::thread::sleep(Duration::from_millis(100));
    hub.release_replies();
    assert!(opener.join().unwrap().is_ok(), "bound on the new session");
    assert_eq!(core.link_status().phase, LinkPhase::Bound);
    core.link_close();
}

/// Fails the first `fail` connects with `error`, then dials for real.
struct Flaky {
    fail: usize,
    error: fn() -> LinkError,
    connects: std::sync::atomic::AtomicUsize,
}

impl WsConnector for Flaky {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        if self.connects.fetch_add(1, Ordering::SeqCst) < self.fail {
            return Err((self.error)());
        }
        TcpConnector::default().connect(url, protocol)
    }
}

fn connect_flaky(
    core: &DspiCore,
    hub: &FakeHub,
    flaky: &Arc<Flaky>,
    divisor: u32,
) -> Result<LinkHello> {
    core.link_connect_with(
        hub.url(),
        "wp-lk4".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            connector: Arc::clone(flaky) as Arc<dyn WsConnector>,
            backoff_divisor: divisor,
            ..Tuning::default()
        },
    )
}

#[test]
fn wp_lk4_the_first_dial_keeps_trying_until_the_hub_answers() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let flaky = Arc::new(Flaky {
        fail: 3,
        error: || LinkError::Timeout,
        connects: Default::default(),
    });
    // Production's backoff: 0.5 + 1 + 2 s before the fourth try
    let started = Instant::now();
    connect_flaky(&core, &hub, &flaky, 1).expect("connected on the fourth try");
    assert_eq!(flaky.connects.load(Ordering::SeqCst), 4);
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    wait_until("the device list", || !core.link_devices().is_empty());
    core.link_close();
}

#[test]
fn wp_lk4_disconnect_stops_a_first_dial_that_keeps_failing() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let flaky = Arc::new(Flaky {
        fail: usize::MAX,
        error: || LinkError::Io("connection refused".into()),
        connects: Default::default(),
    });
    let dialer = {
        let (core, flaky) = (Arc::clone(&core), Arc::clone(&flaky));
        let url = hub.url();
        std::thread::spawn(move || {
            let started = Instant::now();
            let r = core.link_connect_with(
                url,
                "wp-lk4".into(),
                "test".into(),
                String::new(),
                String::new(),
                Tuning {
                    connector: flaky as Arc<dyn WsConnector>,
                    ..Tuning::default()
                },
            );
            (r, started.elapsed())
        })
    };
    wait_until("a few tries", || flaky.connects.load(Ordering::SeqCst) >= 2);
    let closing = Instant::now();
    core.link_close();
    assert!(
        closing.elapsed() < Duration::from_millis(500),
        "link_close waited"
    );
    let (r, took) = dialer.join().unwrap();
    assert!(r.is_err(), "{r:?}");
    assert!(
        took < Duration::from_secs(10),
        "stopped, not the whole budget: {took:?}"
    );
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    let tries = flaky.connects.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        flaky.connects.load(Ordering::SeqCst),
        tries,
        "no try after the close"
    );
}

#[test]
fn wp_lk4_a_refusal_is_not_tried_again() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let flaky = Arc::new(Flaky {
        fail: usize::MAX,
        error: || LinkError::HandshakeRefused {
            status: 403,
            msg: "forbidden".into(),
        },
        connects: Default::default(),
    });
    assert!(connect_flaky(&core, &hub, &flaky, 1).is_err());
    assert_eq!(flaky.connects.load(Ordering::SeqCst), 1);
}

#[test]
fn wp_lk4_a_write_that_needs_the_probe_waits_for_it_and_never_goes_out_unknown() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    slow.delay_ms.store(30, Ordering::SeqCst);
    select(&core).unwrap();
    assert!(
        core.lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.pending_probe.is_some())
    );
    // psychoacoustic bass is a probed feature (the fake firmware lacks it)
    let set = dspi_proto::registry::by_path("bass.on")
        .unwrap()
        .set
        .unwrap();
    let r = core.write_param("bass.on".into(), vec![], crate::ParamValue::Bool(true));
    let features = core
        .lock()
        .conn()
        .unwrap()
        .session()
        .unwrap()
        .capabilities()
        .features
        .len();
    assert!(features > 0, "answered after the probe completed");
    assert_eq!(
        r,
        Err(DspiError::Unsupported),
        "refused in the core's words"
    );
    assert_eq!(
        ran(&hub, set),
        0,
        "nothing the device would reject went out"
    );
    // The typed effect setter waits the same way
    assert_eq!(core.set_psybass_enabled(true), Err(DspiError::Unsupported));
    assert_eq!(ran(&hub, set), 0);
    core.link_close();
}

#[test]
fn wp_lk4_a_write_that_races_the_probe_is_refused_with_its_reason() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    slow.delay_ms.store(30, Ordering::SeqCst);
    select(&core).unwrap();
    // A path that reaches the session without waiting (as a flow of its own would)
    let mut inner = core.lock();
    let session = inner.conn().unwrap().session().unwrap();
    let e = session
        .write("bass.on", &[], dspi_proto::value::Value::Bool(true))
        .unwrap_err();
    assert!(
        matches!(e, dspi_session::WriteError::Unprobed { .. }),
        "{e:?}"
    );
    let d = DspiError::from(e);
    assert!(
        matches!(&d, DspiError::Link { code, .. } if code == "still_reading"),
        "{d:?}"
    );
    drop(inner);
    core.link_close();
}

/// Records the text frames sent, in order.
#[derive(Default)]
struct Sent(Mutex<Vec<String>>);

struct RecordingConnector(Arc<Sent>);

impl WsConnector for RecordingConnector {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        let tx: Box<dyn WsSender> = Box::new(RecordingSender(tx, Arc::clone(&self.0)));
        Ok((tx, rx))
    }
}

struct RecordingSender(Box<dyn WsSender>, Arc<Sent>);

impl WsSender for RecordingSender {
    fn send(&self, msg: WsMessage) -> io::Result<()> {
        if let WsMessage::Text(t) = &msg {
            guard(&self.1.0).push(t.clone());
        }
        self.0.send(msg)
    }
    fn ping(&self) -> io::Result<()> {
        self.0.ping()
    }
    fn close(&self, code: u16) {
        self.0.close(code);
    }
}

#[test]
fn wp_lk4_the_meter_poll_is_asked_for_before_the_snapshot() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let sent = Arc::new(Sent::default());
    core.link_connect_with(
        hub.url(),
        "wp-lk4".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            connector: Arc::new(RecordingConnector(Arc::clone(&sent))),
            ..Tuning::default()
        },
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    select(&core).unwrap();
    let texts = guard(&sent.0).clone();
    let at = |what: &str| texts.iter().position(|t| t.contains(what));
    let (subscribe, snapshot) = (at("poll.subscribe"), at("device.snapshot"));
    assert!(subscribe.is_some() && snapshot.is_some(), "{texts:?}");
    assert!(subscribe < snapshot, "the poll first: {texts:?}");
    wait_until("live meters", || {
        core.fetch_status().is_ok_and(|s| !s.stale)
    });
    core.link_close();
}
