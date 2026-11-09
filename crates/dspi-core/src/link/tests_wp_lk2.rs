//! WP LK2: meters over DSPi Link do not wait for the core lock, and the
//! start-up reads are pipelined.

use std::sync::mpsc;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
use dspi_proto::generated::opcodes as op;

use super::*;

const WAIT: Duration = Duration::from_secs(8);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The fake ESP32 bridge (max_inflight 4, polls), open access.
fn bridge() -> FakeHub {
    bridge_with(4)
}

fn bridge_with(max_inflight: u32) -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::None,
        max_inflight,
        ..HubConfig::bridge()
    })
    .unwrap()
}

/// Connected and bound, without `fetch_all`.
fn selected(hub: &FakeHub) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-lk2".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    // WP LK4: the bind completes its probe in the background; settle it
    core.ensure_probed();
    core
}

/// Hold the core lock on another thread until the returned sender is used
/// or dropped.
fn hold_lock(core: &Arc<DspiCore>) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let c = Arc::clone(core);
    let t = std::thread::spawn(move || {
        let _guard = c.lock();
        held_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    held_rx.recv().unwrap();
    (release_tx, t)
}

#[test]
fn wp_lk2_the_meter_feed_starts_and_runs_while_the_core_lock_is_held() {
    let hub = bridge();
    let core = selected(&hub);
    // As a long fetch_all or cs_fetch would: from right after the open
    let (release, holder) = hold_lock(&core);
    let start = Instant::now();
    let mut live = None;
    while start.elapsed() < WAIT {
        let s = core.fetch_status().unwrap();
        assert!(start.elapsed() < Duration::from_secs(2), "a call waited");
        if !s.stale {
            live = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let live = live.expect("subscribed and fed without the core lock");
    // The fake's meter block: silence, two live inputs
    assert_eq!(live.active_input_channels, 2, "{live:?}");
    drop(release);
    holder.join().unwrap();
    core.link_close();
}

#[test]
fn wp_lk2_the_meter_path_follows_the_connection() {
    let hub = bridge();
    let core = selected(&hub);
    assert!(core.meter_path().is_some(), "recorded at the bind");
    let n = core.get_state().num_channels;
    assert_eq!(core.meter_path().unwrap().num_channels, n);
    core.disconnect();
    assert!(core.meter_path().is_none(), "cleared with the connection");
    assert!(matches!(core.fetch_status(), Err(DspiError::Disconnected)));
    core.link_close();
}

#[test]
fn wp_lk2_without_a_side_channel_the_meters_still_read_under_the_lock() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    assert!(core.meter_path().is_none());
    let (release, holder) = hold_lock(&core);
    let (tx, rx) = mpsc::channel();
    let c = Arc::clone(&core);
    let reader = std::thread::spawn(move || tx.send(c.fetch_status().is_ok()).unwrap());
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "a USB-style read waits for the lock, as before"
    );
    drop(release);
    assert!(rx.recv_timeout(WAIT).unwrap());
    reader.join().unwrap();
    holder.join().unwrap();
}

#[test]
fn wp_lk2_start_up_reads_are_pipelined_up_to_max_inflight() {
    let hub = bridge();
    let core = selected(&hub);
    let base = hub.log().len();
    hub.hold_replies();
    let c = Arc::clone(&core);
    let fetch = std::thread::spawn(move || c.fetch_all());
    // The first batch: four reads out before any answer
    wait_until("four reads on the wire", || hub.log().len() >= base + 4);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        hub.log().len(),
        base + 4,
        "never past the hub's max_inflight"
    );
    hub.release_replies();
    fetch.join().unwrap().unwrap();
    let ops: Vec<(u8, u16)> = hub
        .log()
        .iter()
        .skip(base)
        .map(|(_, _, r, v)| (*r, *v))
        .collect();
    // The order of the one-at-a-time reads (USB, MIDI) is kept
    let mut want = vec![
        (op::REQ_GET_CORE1_MODE, 0),
        (op::REQ_GET_SUBHARM_SOLO, 0),
        (op::REQ_GET_DAC_HW_MUTE_CONFIG, 0),
        (op::REQ_GET_CTRL_IFACE_STATUS, 0),
    ];
    if ops.contains(&(op::REQ_GET_UART_CONFIG, 0)) {
        want.extend([(op::REQ_GET_UART_CONFIG, 0), (op::REQ_GET_I2C_CONFIG, 0)]);
    }
    want.push((op::REQ_PRESET_GET_DIR, 0));
    want.extend((0..10).map(|slot| (op::REQ_PRESET_GET_NAME, slot)));
    assert_eq!(ops.get(..want.len()), Some(&want[..]), "{ops:02X?}");
    core.link_close();
}

#[test]
fn wp_lk2_pipelined_reads_give_the_mirror_one_at_a_time_reads_give() {
    let read = |max_inflight| {
        let hub = bridge_with(max_inflight);
        let core = selected(&hub);
        core.fetch_all().unwrap();
        let state = core.get_state();
        let log: Vec<(u8, u16)> = hub.log().iter().map(|(_, _, r, v)| (*r, *v)).collect();
        core.link_close();
        (state, log)
    };
    // max_inflight 1: the transport sends one at a time, as over USB
    let (piped, piped_log) = read(4);
    let (plain, plain_log) = read(1);
    assert_eq!(piped, plain);
    assert_eq!(piped_log, plain_log, "the same reads, in the same order");
}

/// Live meters within `WAIT`, each call answered at once.
fn live_meters(core: &DspiCore) -> SystemStatus {
    let start = Instant::now();
    loop {
        let t = Instant::now();
        let s = core.fetch_status().unwrap();
        assert!(t.elapsed() < Duration::from_millis(500), "a call waited");
        if !s.stale {
            return s;
        }
        assert!(start.elapsed() < WAIT, "no live meter");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_lk2_a_resync_re_read_does_not_stop_the_meters() {
    let hub = bridge();
    let core = selected(&hub);
    core.fetch_all().unwrap();
    live_meters(&core);
    // WP LK4: the completed probe's refresh, seen first
    let _ = core.process_notifications();
    // The re-read after a RESYNC waits for the hub; the poll feed goes on
    hub.hold_replies();
    let reads = hub.log().len();
    hub.resync();
    let c = Arc::clone(&core);
    let reread = std::thread::spawn(move || {
        let deadline = Instant::now() + WAIT;
        while c.process_notifications().flags & crate::consts::NOTIFY_REFRESHED == 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    wait_until("the re-read to start", || hub.log().len() > reads);
    assert!(core.try_lock().is_none(), "the re-read holds the core lock");
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        live_meters(&core);
    }
    hub.release_replies();
    reread.join().unwrap();
    core.link_close();
}
