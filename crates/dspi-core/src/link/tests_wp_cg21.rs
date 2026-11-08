//! WP CG21: a Stats tick's commands, counted on the fake hub. Android's fast poll
//! measured 3 per tick (50 a second over Link); the Mac's sends 1.

use std::time::{Duration, Instant};

use dspi_link::fakehub::{FakeHub, HubConfig, req};

use super::*;

const TICKS: usize = 20;
/// 448 of 512 frames: at this target the hub's 96-frame rings read as too low.
const FAR_TARGET: u16 = 448;

fn stats_hub() -> FakeHub {
    FakeHub::start(HubConfig {
        stats: true,
        ..HubConfig::default()
    })
    .unwrap()
}

fn bound(hub: &FakeHub) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "wp-cg21".into(),
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
    core.fetch_all().unwrap();
    core
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The commands `read` sends: `(bRequest)` in order.
fn commands(hub: &FakeHub, read: impl FnOnce()) -> Vec<u8> {
    let before = hub.log().len();
    read();
    hub.log()[before..].iter().map(|c| c.2).collect()
}

fn band(core: &DspiCore) -> FillBand {
    core.fetch_stats_tick().unwrap().fills.unwrap()[0].band
}

/// The acceptance measure: the old shape's count, and the new one's.
#[test]
fn wp_cg21_a_tick_is_one_command() {
    let hub = stats_hub();
    let core = bound(&hub);

    let before = commands(&hub, || {
        core.fetch_buffer_stats().unwrap();
        core.fetch_output_fills().unwrap();
    });
    assert_eq!(
        before,
        [
            req::GET_BUFFER_STATS,
            req::OUTPUT_LATENCY,
            req::GET_BUFFER_STATS
        ],
        "the two calls this replaces"
    );

    // The first tick learns the target
    let first = commands(&hub, || {
        core.fetch_stats_tick().unwrap();
    });
    assert_eq!(first, [req::GET_BUFFER_STATS, req::OUTPUT_LATENCY]);

    for tick in 0..TICKS {
        let sent = commands(&hub, || {
            let t = core.fetch_stats_tick().unwrap();
            assert_eq!(t.buffers.num_spdif, 1);
            assert_eq!(t.fills.unwrap().len(), 1);
        });
        assert_eq!(sent, [req::GET_BUFFER_STATS], "tick {tick}");
    }
}

#[test]
fn wp_cg21_an_own_latency_write_rereads_the_target_once() {
    let hub = stats_hub();
    let core = bound(&hub);
    assert_eq!(band(&core), FillBand::Good);

    core.set_output_latency(7).unwrap();
    let mut read = None;
    let sent = commands(&hub, || read = Some(band(&core)));
    assert_eq!(sent, [req::GET_BUFFER_STATS, req::OUTPUT_LATENCY]);
    assert_eq!(read, Some(FillBand::Warn), "judged by the new target");

    let sent = commands(&hub, || {
        band(&core);
    });
    assert_eq!(sent, [req::GET_BUFFER_STATS]);
}

/// The firmware does not notify a latency change: the 2 s health read brings it in.
#[test]
fn wp_cg21_another_clients_change_arrives_with_the_health_read() {
    let hub = stats_hub();
    let core = bound(&hub);
    assert_eq!(band(&core), FillBand::Good);

    hub.set_ring_target(FAR_TARGET, false);
    let sent = commands(&hub, || {
        assert_eq!(band(&core), FillBand::Good, "nothing says so yet");
    });
    assert_eq!(sent, [req::GET_BUFFER_STATS]);

    let health = core.fetch_ring_health().unwrap().unwrap();
    assert_eq!(health.target_frames, FAR_TARGET);
    let mut read = None;
    let sent = commands(&hub, || read = Some(band(&core)));
    assert_eq!(sent, [req::GET_BUFFER_STATS], "no second read for it");
    assert_eq!(read, Some(FillBand::Warn));
}

#[test]
fn wp_cg21_a_change_still_applying_is_read_each_tick_until_it_lands() {
    let hub = stats_hub();
    let core = bound(&hub);
    hub.set_ring_target(96, true);
    core.fetch_ring_health().unwrap();
    for _ in 0..3 {
        let sent = commands(&hub, || {
            band(&core);
        });
        assert_eq!(sent, [req::GET_BUFFER_STATS, req::OUTPUT_LATENCY]);
    }
    hub.set_ring_target(FAR_TARGET, false);
    assert_eq!(band(&core), FillBand::Warn);
    let sent = commands(&hub, || {
        band(&core);
    });
    assert_eq!(sent, [req::GET_BUFFER_STATS]);
}

#[test]
fn wp_cg21_a_resync_invalidates() {
    let hub = stats_hub();
    let core = bound(&hub);
    assert_eq!(band(&core), FillBand::Good);
    hub.set_ring_target(FAR_TARGET, false);
    hub.resync();
    wait_until("the invalidation to be seen", || {
        core.process_notifications();
        band(&core) == FillBand::Warn
    });
}

#[test]
fn wp_cg21_a_link_reattach_invalidates() {
    let hub = stats_hub();
    let core = bound(&hub);
    assert_eq!(band(&core), FillBand::Good);
    hub.set_ring_target(FAR_TARGET, false);
    hub.drop_connections();
    wait_until("the device back, judged by its target", || {
        core.fetch_stats_tick()
            .ok()
            .and_then(|t| t.fills)
            .is_some_and(|f| f[0].band == FillBand::Warn)
    });
}
