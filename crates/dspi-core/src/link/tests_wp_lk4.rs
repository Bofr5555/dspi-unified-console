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
fn wp_lk4_meters_are_live_before_link_select_device_returns() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    // About forty reads at 60 ms each: the select takes seconds
    slow.delay_ms.store(60, Ordering::SeqCst);
    let returned = Arc::new(AtomicBool::new(false));
    let opener = {
        let (core, returned) = (Arc::clone(&core), Arc::clone(&returned));
        std::thread::spawn(move || {
            let r = select(&core);
            returned.store(true, Ordering::SeqCst);
            r
        })
    };
    let mut live_during = false;
    while !returned.load(Ordering::SeqCst) {
        if core.fetch_status().is_ok_and(|s| !s.stale) {
            live_during = !returned.load(Ordering::SeqCst);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(live_during, "a live meter while the select ran");
    opener.join().unwrap().unwrap();
    // The device list's size was right: no second subscription for the image's
    let n = core.get_state().num_channels;
    assert_eq!(core.meter_path().unwrap().num_channels, n);
    assert!(!core.fetch_status().unwrap().stale);
    core.link_close();
}

#[test]
fn wp_lk4_a_failed_select_leaves_no_meter_path() {
    let hub = bridge_with(4);
    let core = DspiCore::new();
    let slow = Arc::new(SlowCommands::default());
    connect_slow(&core, &hub, &slow);
    slow.delay_ms.store(60, Ordering::SeqCst);
    let opener = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || select(&core))
    };
    wait_until("the early feed", || core.meter_path().is_some());
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
    core.disconnect();
    // A new select (not a re-attach) never reuses: the user chose again
    select(&core).unwrap();
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

    // A stall: no frame for longer than FEED_LOST
    let now = Instant::now();
    {
        let mut feed = guard(&side.feed);
        feed.latest.remove(&METER_SLOT);
        feed.meter_subscribed_at = now.checked_sub(FEED_LOST * 2);
        feed.meter_asked = Some(now);
    }
    // Drain nothing new into the past: the hub's frames arrive as they do
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
    let now = Instant::now();
    {
        let mut feed = guard(&side.feed);
        feed.latest.remove(&METER_SLOT);
        feed.meter_subscribed_at = now.checked_sub(FEED_LOST * 2);
        feed.meter_asked = Some(now);
    }
    side.maintain(now);
    assert_eq!(guard(&side.feed).meter_hz, full);
    assert!(guard(&side.feed).meter_slow_since.is_none());
    core.link_close();
}
