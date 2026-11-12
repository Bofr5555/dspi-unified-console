//! WP LKCLOSE: the hub's close code decides how a session ended, also when a write
//! fails before the reader has read the close frame. The hub sends its close and
//! drops the socket, so the client's next write can fail while that frame waits
//! unread; a close 4000 taken for a lost link would reconnect, which spec 5 forbids.
//! The socket here is the real one, with its two halves held to an order by the test.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::{TcpConnector, WsConnector, WsMessage, WsPair, WsReceiver, WsRecv, WsSender};
use dspi_link::{should_reconnect, LinkClient, LinkError, SessionConfig};

use crate::support::*;

/// What the test holds: the reader's end of the session, and the writes.
#[derive(Default)]
struct Order {
    /// The reader's `Closed` waits while this is set.
    hold_close: Mutex<bool>,
    released: Condvar,
    /// The reader has its `Closed` and is waiting.
    close_read: AtomicBool,
    /// Every write fails, as on a socket the hub has dropped.
    writes_fail: AtomicBool,
}

impl Order {
    fn hold(&self, on: bool) {
        *self.hold_close.lock().unwrap() = on;
        self.released.notify_all();
    }

    fn wait_while_held(&self) {
        self.close_read.store(true, Ordering::SeqCst);
        let mut held = self.hold_close.lock().unwrap();
        while *held {
            held = self.released.wait(held).unwrap();
        }
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

impl Receiver {
    fn after(&self, r: WsRecv) -> WsRecv {
        if matches!(r, WsRecv::Closed { .. }) {
            self.1.wait_while_held();
        }
        r
    }
}

impl WsReceiver for Receiver {
    fn recv(&mut self) -> WsRecv {
        let r = self.0.recv();
        self.after(r)
    }
    fn recv_within(&mut self, max: Duration) -> WsRecv {
        let r = self.0.recv_within(max);
        self.after(r)
    }
}

struct Connector(Arc<Order>);

impl WsConnector for Connector {
    fn connect(&self, url: &str, protocol: &str) -> Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        Ok((Box::new(Sender(tx, Arc::clone(&self.0))), Box::new(Receiver(rx, Arc::clone(&self.0)))))
    }
}

fn session() -> (FakeHub, LinkClient, Arc<Order>) {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let order = Arc::new(Order::default());
    let (c, _) = LinkClient::connect_with(
        &Connector(Arc::clone(&order)),
        &hub.url(),
        &opts(None),
        None,
        &SessionConfig::default(),
    )
    .unwrap();
    (hub, c, order)
}

/// The close code, once the session has one.
fn ended(c: &LinkClient) -> Option<u16> {
    assert!(wait(|| c.closed_reason().is_some()), "the session did not end");
    c.closed_reason().unwrap().0
}

/// The failure seen in the Qt smoke (`qp13`, phases `bound > reconnecting`): the write
/// fails while the close frame is still unread.
#[test]
fn wp_lkclose_a_write_failing_before_the_close_is_read_keeps_its_code() {
    let (hub, c, order) = session();
    order.hold(true);
    hub.close_all(4000);
    assert!(wait(|| order.close_read.load(Ordering::SeqCst)), "the close never came");
    order.writes_fail.store(true, Ordering::SeqCst);
    c.poll_unsubscribe(0, &[1]);
    let failed = c.refresh_devices();
    assert!(failed.is_err(), "{failed:?}");
    // Unusable at once (the framing is unknown, F7), and how it ended is not yet known
    assert!(c.is_closed());
    assert_eq!(c.closed_reason(), None);
    order.hold(false);
    assert_eq!(ended(&c), Some(4000));
    assert!(!should_reconnect(c.closed_reason().unwrap().0));
}

#[test]
fn wp_lkclose_a_write_failing_after_the_close_is_read_keeps_its_code() {
    let (hub, c, order) = session();
    hub.close_all(4000);
    assert_eq!(ended(&c), Some(4000));
    order.writes_fail.store(true, Ordering::SeqCst);
    c.poll_unsubscribe(0, &[1]);
    assert!(c.refresh_devices().is_err());
    assert_eq!(c.closed_reason().unwrap().0, Some(4000));
}

/// With nothing from the hub to read, a failed write still ends the session, with no
/// code: a lost link, which reconnects.
#[test]
fn wp_lkclose_a_failed_write_alone_ends_the_session_as_lost() {
    let (_hub, c, order) = session();
    order.writes_fail.store(true, Ordering::SeqCst);
    c.poll_unsubscribe(0, &[1]);
    assert_eq!(ended(&c), None);
    assert!(should_reconnect(None));
}
