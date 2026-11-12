//! WP TM26: a hub that holds its accepts until its device is in (hw-1 M2).
//!
//! `uconsole hub` listened about 0.3 s before the USB device attached, and a client in that
//! window was told "No DSPi found". With `NetworkConfig::hold_accepts` the port is bound
//! at once and clients wait in the backlog until the embedder calls `start_accepting`.
//! The attach here is slowed down on purpose. Loopback only.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dspi_hub::server::Advertise;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, LinkClient};
use dspi_transport::FakeDevice;

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-tm26").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn open_hub(hold_accepts: bool) -> Hub {
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::Off,
            hold_accepts,
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("Slow hub")
    })
    .unwrap()
}

/// Connect in the background, and report how many devices the first `hello` listed.
fn client_in_background(port: u16) -> std::thread::JoinHandle<usize> {
    std::thread::spawn(move || {
        let options = ConnectOptions {
            client_name: "early client".into(),
            app: "wp_tm26".into(),
            app_version: "0".into(),
            token: None,
        };
        let (client, state) = LinkClient::connect(
            &TcpConnector::default(),
            &format!("ws://127.0.0.1:{port}/dspi/v1"),
            &options,
            None,
        )
        .unwrap();
        assert!(matches!(state, AuthState::Ready), "{state:?}");
        client.devices().len()
    })
}

#[test]
fn wp_tm26_an_early_client_waits_for_the_slow_attach() {
    let hub = open_hub(true);
    let net = hub.network().unwrap();
    let early = client_in_background(net.port());

    // The slow attach: the client is in the backlog, not in the hub.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(net.connections(), 0, "accepted before the device was in");
    assert!(!early.is_finished());
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    net.start_accepting();

    assert_eq!(
        early.join().unwrap(),
        1,
        "the first answer lists the device"
    );
    hub.stop();
}

/// Without the hold, nothing changes: the hub accepts at once, device or not.
#[test]
fn wp_tm26_without_the_hold_the_hub_accepts_at_once() {
    let hub = open_hub(false);
    let net = hub.network().unwrap();
    let early = client_in_background(net.port());
    assert_eq!(early.join().unwrap(), 0, "answered before any attach");
    // Calling it anyway changes nothing.
    net.start_accepting();
    hub.stop();
}
