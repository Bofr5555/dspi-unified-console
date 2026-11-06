//! A dark network without touching the hub or the host's network: the harness
//! dials through this, and a stall stops forwarding while both TCP connections
//! stay up. Bytes wait in the kernel, as they do across a real Wi-Fi outage.
//! What a physical pull adds (the radio, DHCP, the phone's own reaction) stays
//! manual.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(10);
/// A pump inside its read when a stall starts has come out of it by now.
const PARK: Duration = Duration::from_millis(30);

struct Shared {
    up_stalled: AtomicBool,
    down_stalled: AtomicBool,
    stop: AtomicBool,
    conns: Mutex<Vec<(TcpStream, TcpStream)>>,
}

impl Shared {
    fn cut(&self) {
        for (a, b) in self.conns.lock().unwrap().drain(..) {
            let _ = a.shutdown(Shutdown::Both);
            let _ = b.shutdown(Shutdown::Both);
        }
    }
}

pub struct StallProxy {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl StallProxy {
    pub fn start(host: &str, port: u16) -> io::Result<StallProxy> {
        let target =
            (host, port).to_socket_addrs()?.next().ok_or_else(|| io::Error::other("the hub's name has no address"))?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            up_stalled: AtomicBool::new(false),
            down_stalled: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            conns: Mutex::new(Vec::new()),
        });
        let s = Arc::clone(&shared);
        thread::spawn(move || {
            while !s.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let _ = join(client, target, &s);
                    }
                    Err(_) => thread::sleep(TICK),
                }
            }
        });
        Ok(StallProxy { addr, shared })
    }

    pub fn url(&self) -> String {
        format!("ws://{}/dspi/v1", self.addr)
    }

    /// Nothing passes either way, from when this returns.
    pub fn stall(&self, on: bool) {
        self.shared.up_stalled.store(on, Ordering::SeqCst);
        self.shared.down_stalled.store(on, Ordering::SeqCst);
        thread::sleep(PARK);
    }

    /// Requests reach the hub, and its replies wait, from when this returns.
    pub fn stall_replies(&self, on: bool) {
        self.shared.down_stalled.store(on, Ordering::SeqCst);
        thread::sleep(PARK);
    }

    /// Cut every connection, with no close frame.
    pub fn cut(&self) {
        self.shared.cut();
    }
}

impl Drop for StallProxy {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.cut();
    }
}

fn join(client: TcpStream, target: SocketAddr, s: &Arc<Shared>) -> io::Result<()> {
    client.set_nonblocking(false)?;
    let hub = TcpStream::connect_timeout(&target, Duration::from_secs(5))?;
    let _ = client.set_nodelay(true);
    let _ = hub.set_nodelay(true);
    s.conns.lock().unwrap().push((client.try_clone()?, hub.try_clone()?));
    pump(client.try_clone()?, hub.try_clone()?, Arc::clone(s), true)?;
    pump(hub, client, Arc::clone(s), false)
}

fn pump(mut from: TcpStream, mut to: TcpStream, s: Arc<Shared>, up: bool) -> io::Result<()> {
    from.set_read_timeout(Some(TICK))?;
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while !s.stop.load(Ordering::SeqCst) {
            let stalled = if up { &s.up_stalled } else { &s.down_stalled };
            if stalled.load(Ordering::SeqCst) {
                thread::sleep(TICK);
                continue;
            }
            match from.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if to.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        let _ = from.shutdown(Shutdown::Both);
        let _ = to.shutdown(Shutdown::Both);
    });
    Ok(())
}
