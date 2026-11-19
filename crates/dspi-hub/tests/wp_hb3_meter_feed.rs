//! WP HB3's done criterion: C15's meter feed works against the hub's polls, end to end:
//! the client is the real one, `DspiCore` over Link, and the server is the real one (HB1).
//! Alone in its binary, since it measures transfers per second in real time.
#![allow(clippy::arithmetic_side_effects, clippy::result_large_err)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

const WAIT: Duration = Duration::from_secs(20);

/// A hub on its real server, open access on loopback, as C15's client meets one. Its
/// `ws://` URL, and the hub.
fn hub_on_the_network() -> (Hub, String) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hb3-feed").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir)
        }),
        ..HubConfig::in_process("test")
    })
    .unwrap();
    let port = hub.network().unwrap().port();
    (hub, format!("ws://127.0.0.1:{port}/dspi/v1"))
}

// -- The device ------------------------------------------------------------------

/// The demo device, counting reads of the meter block.
struct Counted {
    inner: FakeDevice,
    descriptor: DeviceDescriptor,
    meter_reads: Arc<Mutex<usize>>,
}

fn device(serial: &str) -> (Counted, FakeRemote, Arc<Mutex<usize>>) {
    let inner = FakeDevice::demo();
    let remote = inner.remote();
    let reads = Arc::new(Mutex::new(0));
    (
        Counted {
            inner,
            descriptor: DeviceDescriptor {
                serial: serial.into(),
                bus_id: "test".into(),
                address: 1,
                ..Default::default()
            },
            meter_reads: Arc::clone(&reads),
        },
        remote,
        reads,
    )
}

impl Transport for Counted {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        self.inner.control_in(o, v, l)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        if o == op::REQ_GET_STATUS && v == 9 {
            *self.meter_reads.lock().unwrap() += 1;
        }
        self.inner.control_in_upto(o, v, l)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(o, v, d)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

fn connect(url: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.link_connect(
        url.to_owned(),
        "wp-hb3".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A meter block whose first peak is `peak`, sized as the device's.
fn block(remote: &FakeRemote, peak: u16) -> Vec<u8> {
    // `2 * channels + 7`, from the image's header (spec 7.6).
    let channels = usize::from(remote.image()[2]);
    let mut b = vec![0; 2 * channels + 7];
    b[..2].copy_from_slice(&peak.to_le_bytes());
    b
}

fn first_peak(core: &DspiCore) -> f32 {
    core.fetch_status()
        .ok()
        .and_then(|s| s.peaks.first().copied())
        .unwrap_or(0.0)
}

/// Asks as a UI does, at its own rate, for `for_`.
fn ask_for(cores: &[&DspiCore], for_: Duration) {
    let start = Instant::now();
    while start.elapsed() < for_ {
        for c in cores {
            let _ = c.fetch_status();
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn rate(reads: &Mutex<usize>, cores: &[&DspiCore], for_: Duration) -> f64 {
    let before = *reads.lock().unwrap();
    let start = Instant::now();
    ask_for(cores, for_);
    (*reads.lock().unwrap() - before) as f64 / start.elapsed().as_secs_f64()
}

#[test]
fn wp_hb3_the_meter_feed_works_against_polls() {
    let (hub, url) = hub_on_the_network();
    let (dev, remote, reads) = device("METER");
    hub.attach(Box::new(dev), 0).unwrap();
    remote.set_status(block(&remote, 0x4000));

    // One client: its feed, at the rate it asked for, while it asks far faster.
    let a = connect(&url);
    wait_until("the first client's meters", || first_peak(&a) > 0.0);
    let one = rate(&reads, &[&a], Duration::from_secs(2));

    // A second client: the same read, not another.
    let b = connect(&url);
    wait_until("the second client's meters", || {
        let _ = a.fetch_status();
        first_peak(&b) > 0.0
    });
    let two = rate(&reads, &[&a, &b], Duration::from_secs(2));
    println!("HB3 measured over the real client: one client {one:.1} transfers/s, two {two:.1}");
    assert!((8.0..=17.0).contains(&one), "C15 asks for 15 Hz: {one}");
    assert!(two <= 17.0, "shared, not doubled: {two}");
    assert!(two >= 8.0, "still running: {two}");

    // The feed is live, for both: a change at the device shows.
    remote.set_status(block(&remote, 0x1000));
    let live = |c: &DspiCore| {
        let v = first_peak(c);
        v > 0.0 && (v - first_peak_of(0x1000)).abs() < 0.001
    };
    wait_until("the new peak", || live(&a) && live(&b));

    // Only the second stops asking: its subscription goes, the read stays for the first.
    let idle_b = Instant::now();
    while idle_b.elapsed() < Duration::from_secs(4) {
        let _ = a.fetch_status();
        std::thread::sleep(Duration::from_millis(16));
    }
    let still = rate(&reads, &[&a], Duration::from_secs(1));
    assert!(still >= 8.0, "the first still reads: {still}");

    // Nobody asks: C15 unsubscribes after its idle limit, and the device hears nothing.
    wait_until("the reads to stop", || {
        let before = *reads.lock().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        *reads.lock().unwrap() == before
    });
    std::thread::sleep(Duration::from_millis(500));
    let settled = *reads.lock().unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let idle = *reads.lock().unwrap() - settled;
    println!("HB3 measured over the real client: UIs idle, {idle} transfers/s");
    assert_eq!(idle, 0, "nobody is watching");

    // Asked again, it comes back.
    wait_until("the feed again", || {
        let _ = b.fetch_status();
        *reads.lock().unwrap() > settled
    });
}

/// The peak `fetch_status` reports for a raw Q15 value.
fn first_peak_of(raw: u16) -> f32 {
    f32::from(raw) / 32767.0
}

#[test]
fn wp_hb3_the_meter_feed_comes_back_after_a_replug() {
    let (hub, url) = hub_on_the_network();
    let (dev, remote, _) = device("REPLUG");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    remote.set_status(block(&remote, 0x4000));
    let core = connect(&url);
    wait_until("meters", || first_peak(&core) > 0.0);

    remote.disconnect();
    wait_until("offline", || {
        hub.devices()[0].state == dspi_hub::DeviceState::Offline
    });
    ask_for(&[&core], Duration::from_millis(200));

    let (dev, remote, reads) = device("REPLUG");
    remote.set_status(block(&remote, 0x2000));
    assert_eq!(hub.attach(Box::new(dev), 0).unwrap(), handle);
    wait_until("the new board's meters", || {
        let _ = core.fetch_all();
        (first_peak(&core) - first_peak_of(0x2000)).abs() < 0.001
    });
    assert!(*reads.lock().unwrap() > 0);
}
