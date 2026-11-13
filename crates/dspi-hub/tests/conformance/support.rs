//! The hub under test, a fault proxy in front of it, and helpers.
#![allow(dead_code)]

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::server::{NetworkControl, ServerLimits};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, LinkClient, LinkError, SessionConfig};
use dspi_proto::generated::notify as nfy;
use dspi_transport::{FakeDevice, FakeRemote};

pub const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const WAIT: Duration = Duration::from_secs(10);
/// A change made at the device: a knob.
pub const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;
pub const HUB_NAME: &str = "Conformance hub";

pub fn store_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` and `run-clients.sh` point TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hb7").join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Short enough to wait out, long enough for a loaded build machine.
pub fn quick() -> ServerLimits {
    ServerLimits {
        head_timeout: Duration::from_millis(500),
        auth_timeout: Duration::from_secs(2),
        ping_interval: Duration::from_millis(100),
        dead_peer: Duration::from_millis(1500),
        write_stall: Duration::from_millis(500),
        close_grace: Duration::from_millis(200),
        read_tick: Duration::from_millis(20),
        ..ServerLimits::default()
    }
}

/// What a hub is started with, kept so it can be started again in the same place.
#[derive(Clone)]
pub struct Setup {
    pub access: AccessMode,
    pub limits: ServerLimits,
    pub store: PathBuf,
    pub bind: IpAddr,
    pub port: u16,
    /// How long an unplugged device keeps its handle, if not the hub's default.
    pub offline_grace: Option<Duration>,
}

impl Setup {
    pub fn pin(tag: &str) -> Self {
        Self {
            access: AccessMode::Pin,
            limits: quick(),
            store: store_dir(tag),
            bind: LOOPBACK,
            port: 0,
            offline_grace: None,
        }
    }
}

/// A networked hub serving the demo device at handle 0.
pub struct Bench {
    pub hub: Hub,
    pub net: NetworkControl,
    pub remote: FakeRemote,
    pub setup: Setup,
}

impl Bench {
    pub fn start(setup: Setup) -> Self {
        Self::try_start(setup).unwrap()
    }

    pub fn try_start(setup: Setup) -> Result<Self, dspi_hub::HubError> {
        let defaults = HubConfig::in_process(HUB_NAME);
        let hub = Hub::start(HubConfig {
            offline_grace: setup.offline_grace.unwrap_or(defaults.offline_grace),
            network: Some(NetworkConfig {
                port: setup.port,
                access: setup.access,
                bind: setup.bind,
                limits: setup.limits.clone(),
                ..NetworkConfig::new(setup.store.clone())
            }),
            ..defaults
        })?;
        let device = FakeDevice::demo();
        let remote = device.remote();
        hub.attach(Box::new(device), 0).unwrap();
        let net = hub.network().unwrap();
        let setup = Setup {
            port: net.port(),
            ..setup
        };
        Ok(Self {
            hub,
            net,
            remote,
            setup,
        })
    }

    pub fn pin_hub(tag: &str) -> Self {
        Self::start(Setup::pin(tag))
    }

    pub fn port(&self) -> u16 {
        self.setup.port
    }

    pub fn url(&self) -> String {
        url(self.port())
    }

    /// A new pairing window's PIN, as the hub's UI would show it.
    pub fn pin(&self) -> String {
        self.net.auth().begin_pairing(Instant::now()).unwrap().pin
    }

    /// -- WP HWLK1F -- a PIN whose pairing may be admin (`uconsole hub --pair-admin`);
    /// a plain one gives at most control.
    pub fn admin_pin(&self) -> String {
        self.net
            .auth()
            .begin_pairing_admin(Instant::now())
            .unwrap()
            .pin
    }

    /// Stop, and start again on the same port with the same store: a hub restart.
    pub fn restart(self) -> Self {
        let setup = self.setup.clone();
        self.hub.stop();
        drop(self);
        // The port is the client's address for the hub; give a slow machine a moment.
        let start = Instant::now();
        loop {
            match Self::try_start(setup.clone()) {
                Ok(b) => return b,
                Err(_) if start.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("the hub did not start again: {e}"),
            }
        }
    }

    /// A change made at the device itself (a knob), which every session hears with
    /// origin 0. Flips the last parameter byte the demo image has.
    pub fn knob(&self) {
        let image = self.remote.image();
        let at = image.len() - 8;
        let next = image[at] ^ 0x5A;
        assert!(self.remote.poke(at, &[next], GPIO));
    }

    /// The paired client called `name`, for a revoke.
    pub fn client_id(&self, name: &str) -> Option<u32> {
        self.net
            .auth()
            .clients()
            .into_iter()
            .find(|c| c.name == name)
            .map(|c| c.cid)
    }

    pub fn wait_connections(&self, count: usize) {
        wait_until(&format!("{count} connections"), || {
            self.net.connections() == count
        });
    }
}

pub fn url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}/dspi/v1")
}

pub fn options(name: &str, token: Option<String>) -> ConnectOptions {
    ConnectOptions {
        client_name: name.into(),
        app: "wp_hb7".into(),
        app_version: "0".into(),
        token,
    }
}

pub fn connect(
    url: &str,
    name: &str,
    token: Option<String>,
) -> Result<(LinkClient, AuthState), LinkError> {
    connect_with(url, name, token, &SessionConfig::default())
}

pub fn connect_with(
    url: &str,
    name: &str,
    token: Option<String>,
    cfg: &SessionConfig,
) -> Result<(LinkClient, AuthState), LinkError> {
    LinkClient::connect_with(
        &TcpConnector::default(),
        url,
        &options(name, token),
        None,
        cfg,
    )
}

pub fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A TCP proxy in front of a hub, for the faults a real network has and a real hub
/// cannot be told to make: a held or stalled direction, a cut connection.
pub struct Proxy {
    port: u16,
    shared: Arc<ProxyShared>,
}

#[derive(Default)]
struct ProxyShared {
    /// Hub to client held: the bytes wait in the proxy and the kernel, in order.
    hold_down: AtomicBool,
    /// Client to hub held.
    hold_up: AtomicBool,
    stop: AtomicBool,
    streams: Mutex<Vec<TcpStream>>,
}

impl Proxy {
    pub fn start(hub_port: u16) -> Self {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let shared = Arc::new(ProxyShared::default());
        let s = Arc::clone(&shared);
        std::thread::spawn(move || {
            while !s.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let Ok(hub) = TcpStream::connect((LOOPBACK, hub_port)) else {
                            continue;
                        };
                        client.set_nonblocking(false).unwrap();
                        {
                            let mut list = s.streams.lock().unwrap();
                            list.push(client.try_clone().unwrap());
                            list.push(hub.try_clone().unwrap());
                        }
                        pump(
                            &s,
                            client.try_clone().unwrap(),
                            hub.try_clone().unwrap(),
                            false,
                        );
                        pump(&s, hub, client, true);
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { port, shared }
    }

    pub fn url(&self) -> String {
        url(self.port)
    }

    pub fn hold_down(&self, on: bool) {
        self.shared.hold_down.store(on, Ordering::SeqCst);
    }

    /// Both directions held: a Wi-Fi stall.
    pub fn stall(&self, on: bool) {
        self.shared.hold_down.store(on, Ordering::SeqCst);
        self.shared.hold_up.store(on, Ordering::SeqCst);
    }

    /// Every connection through the proxy reset, as when a route goes away.
    pub fn cut(&self) {
        for s in self.shared.streams.lock().unwrap().drain(..) {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.cut();
    }
}

fn pump(s: &Arc<ProxyShared>, mut from: TcpStream, mut to: TcpStream, down: bool) {
    let s = Arc::clone(s);
    from.set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let held = if down { &s.hold_down } else { &s.hold_up };
        let wait_while_held = || {
            while held.load(Ordering::SeqCst) && !s.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        loop {
            wait_while_held();
            if s.stop.load(Ordering::SeqCst) {
                break;
            }
            match from.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Bytes read just as the hold began wait with the rest.
                    wait_while_held();
                    if to.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        let _ = to.shutdown(Shutdown::Both);
        let _ = from.shutdown(Shutdown::Both);
    });
}
