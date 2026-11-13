//! WP LKCLOSE: after the hub's `close 4000` the core ends the session (`Closed`) and
//! does not reconnect, also when one of its writes fails before the client has read
//! that close frame: the Qt smoke's `qp13`, seen as `bound > reconnecting` about once
//! in 36 runs. The two halves of the real socket are held to that order here.

use std::io;
use std::sync::Condvar;

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::{
    TcpConnector, WsConnector, WsMessage, WsPair, WsReceiver, WsRecv, WsSender,
};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What the test holds: the reader's `Closed`, and whether writes fail.
#[derive(Default)]
struct Order {
    hold_close: Mutex<bool>,
    released: Condvar,
    close_read: AtomicBool,
    writes_fail: AtomicBool,
    connects: std::sync::atomic::AtomicUsize,
}

impl Order {
    fn hold(&self, on: bool) {
        *guard(&self.hold_close) = on;
        self.released.notify_all();
    }

    fn write(&self) -> io::Result<()> {
        if self.writes_fail.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

struct Sender(Box<dyn WsSender>, Arc<Order>);

impl WsSender for Sender {
    fn send(&self, msg: WsMessage) -> io::Result<()> {
        self.1.write()?;
        self.0.send(msg)
    }
    fn ping(&self) -> io::Result<()> {
        self.1.write()?;
        self.0.ping()
    }
    fn close(&self, code: u16) {
        self.0.close(code);
    }
}

struct Receiver(Box<dyn WsReceiver>, Arc<Order>);

impl WsReceiver for Receiver {
    fn recv(&mut self) -> WsRecv {
        let r = self.0.recv();
        if matches!(r, WsRecv::Closed { .. }) {
            self.1.close_read.store(true, Ordering::SeqCst);
            let mut held = guard(&self.1.hold_close);
            while *held {
                held = self
                    .1
                    .released
                    .wait(held)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
        r
    }
}

struct Connector(Arc<Order>);

impl WsConnector for Connector {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        self.0.connects.fetch_add(1, Ordering::SeqCst);
        Ok((
            Box::new(Sender(tx, Arc::clone(&self.0))),
            Box::new(Receiver(rx, Arc::clone(&self.0))),
        ))
    }
}

#[test]
fn wp_lkclose_close_4000_ends_the_session_when_a_write_fails_first() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let order = Arc::new(Order::default());
    let core = DspiCore::new();
    core.link_connect_with(
        hub.url(),
        "wp-lkclose".into(),
        "test".into(),
        String::new(),
        String::new(),
        Tuning {
            connector: Arc::new(Connector(Arc::clone(&order))),
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    wait_until("the device list", || !core.link_devices().is_empty());
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    wait_until("bound", || core.link_status().phase == LinkPhase::Bound);

    // The hub's close is on the socket, unread, when the core's write fails.
    order.hold(true);
    hub.close_all(4000);
    wait_until("the close at the reader", || {
        order.close_read.load(Ordering::SeqCst)
    });
    order.writes_fail.store(true, Ordering::SeqCst);
    assert!(core.set_master_volume(-9.0).is_err());
    order.hold(false);

    wait_until("closed", || core.link_status().phase == LinkPhase::Closed);
    let status = core.link_status();
    assert_eq!(status.close_code, 4000, "{status:?}");
    // Ended: no retry comes after it
    order.writes_fail.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(core.link_status().phase, LinkPhase::Closed);
    assert_eq!(
        order.connects.load(Ordering::SeqCst),
        1,
        "never dialled again"
    );
    core.link_close();
}
