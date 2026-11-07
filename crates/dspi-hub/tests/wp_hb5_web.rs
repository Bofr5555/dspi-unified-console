//! WP HB5: what a hub serves a browser, over real sockets on loopback. The placeholder
//! stands in for a real page; nothing here knows what the page says beyond its being one.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dspi_hub::server::{Advertise, NetworkConfig};
use dspi_hub::web;
use dspi_hub::{Hub, HubConfig};
use dspi_transport::FakeDevice;

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hb5-web").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start(web: bool) -> (Hub, u16) {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            web,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::Off,
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("Web hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let port = hub.network().unwrap().port();
    (hub, port)
}

/// The status, head and body of one request.
fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(tcp, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
    let mut bytes = Vec::new();
    tcp.read_to_end(&mut bytes).unwrap();
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (code, head, bytes[split + 4..].to_vec())
}

fn upgrade_status(port: u16, origin: &str) -> u16 {
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        tcp,
        "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\
         Origin: {origin}\r\n\r\n"
    )
    .unwrap();
    let mut first = [0u8; 12];
    tcp.read_exact(&mut first).unwrap();
    String::from_utf8_lossy(&first[9..]).trim().parse().unwrap()
}

#[test]
fn wp_hb5_the_page_is_served_on_the_hubs_port_when_web_is_on() {
    let (hub, port) = start(true);
    for path in ["/", "/index.html", "/?from=link"] {
        let (code, head, body) = get(port, path);
        assert_eq!(code, 200, "{path}");
        assert_eq!(body, web::asset("/").unwrap().body, "{path}");
        let head = head.to_ascii_lowercase();
        assert!(head.contains("content-type: text/html"), "{head}");
        assert!(head.contains("x-content-type-options: nosniff"), "{head}");
        assert!(head.contains("cache-control: no-cache"), "{head}");
    }
    hub.stop();
}

#[test]
fn wp_hb5_nothing_is_served_with_web_off() {
    let (hub, port) = start(false);
    for path in ["/", "/index.html"] {
        assert_eq!(get(port, path).0, 404, "{path}");
    }
    assert_eq!(
        get(port, "/dspi/v1/info").0,
        200,
        "the protocol's own stays"
    );
    hub.stop();
}

#[test]
fn wp_hb5_the_page_stops_when_sharing_does() {
    let (hub, port) = start(true);
    assert_eq!(get(port, "/").0, 200);
    hub.stop();
    assert!(
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err(),
        "the port must be closed once the hub is gone"
    );
}

#[test]
fn wp_hb5_only_listed_files_and_the_protocols_paths_answer() {
    let (hub, port) = start(true);
    for path in [
        "/../Cargo.toml",
        "/..%2fCargo.toml",
        "/etc/passwd",
        "//index.html",
        "/dspi/",
        "/dspi/v1/nothing",
    ] {
        assert_eq!(get(port, path).0, 404, "{path}");
    }
    let (code, _, body) = get(port, "/dspi/v1/info");
    assert_eq!(code, 200);
    let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["hub"]["name"], "Web hub");
    hub.stop();
}

#[test]
fn wp_hb5_the_pages_own_origin_reaches_the_socket_and_others_do_not() {
    let (hub, port) = start(true);
    assert_eq!(
        upgrade_status(port, &format!("http://127.0.0.1:{port}")),
        101
    );
    assert_eq!(
        upgrade_status(port, &format!("http://localhost:{port}")),
        101
    );
    assert_eq!(upgrade_status(port, "http://evil.example"), 403);
    assert_eq!(
        upgrade_status(port, &format!("https://127.0.0.1:{port}")),
        403
    );
    hub.stop();
}

/// For a real browser: serves the demo device on loopback, with no advertising, for
/// `DSPI_HB5_SERVE_SECS` (default 60). `DSPI_HB5_PORT` picks the port, `DSPI_HB5_MDNS_NAME`
/// the `.local` name a browser can be pointed at. Docs in HUB.md section 9.
#[test]
#[ignore = "by hand, with a browser"]
fn wp_hb5_serve_the_placeholder_for_a_browser() {
    let var = |name| std::env::var(name).ok();
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: var("DSPI_HB5_PORT").map_or(0, |p| p.parse().unwrap()),
            web: true,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::Off,
            mdns_name: var("DSPI_HB5_MDNS_NAME"),
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("Browser test hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    println!(
        "serving on http://127.0.0.1:{}/",
        hub.network().unwrap().port()
    );
    let secs = var("DSPI_HB5_SERVE_SECS").map_or(60, |s| s.parse().unwrap());
    std::thread::sleep(Duration::from_secs(secs));
    hub.stop();
}
