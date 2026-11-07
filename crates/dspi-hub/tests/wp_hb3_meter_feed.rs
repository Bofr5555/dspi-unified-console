//! WP HB3's done criterion: C15's meter feed works against the hub's polls. The client
//! here is the real one, `DspiCore` over Link. HB1's server is not in this branch, so a
//! small server below carries Link frames to `HubHandle`'s interfaces and no further: it
//! decides nothing, as every answer comes from the hub. Alone in its binary, since it
//! measures transfers per second in real time.
#![allow(clippy::arithmetic_side_effects, clippy::result_large_err)]

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::session::Pop;
use dspi_hub::{
    Completion, Hub, HubConfig, HubHandle, Outbound, POLL_BUDGET_BPS, POLL_MAX_HZ, Role,
};
use dspi_link::client::SUB_PROTOCOL;
use dspi_link::frames::{Frame, NotifyFrame, PollFrame};
use dspi_link::messages::PollSpec;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};
use serde_json::{Value, json};
use tungstenite::handshake::server::{Request, Response};
use tungstenite::{Error as WsError, Message, WebSocket};

const WAIT: Duration = Duration::from_secs(20);

// -- The stand-in for HB1's server ---------------------------------------------

fn reply(id: &Value, mut body: Value) -> String {
    body["t"] = json!("ok");
    body["id"] = id.clone();
    body.to_string()
}

fn refuse(id: &Value, code: &str) -> String {
    json!({"t": "err", "id": id, "code": code}).to_string()
}

fn control(hub: &HubHandle, session: &dspi_hub::Session, text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    let id = v["id"].clone();
    let handle = v["handle"].as_u64().unwrap_or(0) as u8;
    Some(match v["t"].as_str()? {
        "hello" => json!({
            "t": "hello",
            "proto": {"major": 1, "minor": 2},
            "hub": {"id": "00000000-0000-4000-8000-000000000003", "name": hub.name(), "kind": "console", "version": "test"},
            "auth": "none",
            "caps": ["cmd", "notify", "poll", "snapshot"],
            "limits": {"max_frame": 65536, "max_payload": 8192, "max_inflight": 8,
                       "poll_max_hz": POLL_MAX_HZ, "poll_budget_bps": POLL_BUDGET_BPS},
        })
        .to_string(),
        "auth.pair" => reply(
            &id,
            json!({"session": session.id(), "role": "admin", "policy": {"denied": []}}),
        ),
        "device.list" => {
            let devices: Vec<Value> = hub.devices().iter().map(|d| d.to_json()).collect();
            reply(&id, json!({"devices": devices}))
        }
        "poll.subscribe" => {
            let specs: Vec<PollSpec> = serde_json::from_value(v["polls"].clone()).ok()?;
            match hub.poll_subscribe(session, handle, &specs) {
                Ok(granted) => reply(&id, json!({"granted": granted})),
                Err(e) => refuse(&id, e.code()),
            }
        }
        "poll.unsubscribe" => {
            let slots: Vec<u8> = serde_json::from_value(v["slots"].clone()).ok()?;
            hub.poll_unsubscribe(session, handle, &slots);
            reply(&id, json!({}))
        }
        _ => refuse(&id, "unknown_type"),
    })
}

fn pump(ws: &mut WebSocket<TcpStream>, session: &dspi_hub::Session, poll_seq: &mut u16) -> bool {
    while let Pop::Item(item) = session.outbox().pop(Duration::ZERO) {
        let msg = match item {
            Outbound::Notify {
                seq,
                handle,
                origin,
                packet,
            } => Frame::Notify(NotifyFrame {
                seq,
                handle,
                origin,
                packet: packet.to_vec(),
            }),
            Outbound::Resync { handle, reason } => Frame::Resync { handle, reason },
            Outbound::Poll {
                handle,
                slot,
                payload,
            } => {
                *poll_seq = poll_seq.wrapping_add(1);
                Frame::Poll(PollFrame {
                    seq: *poll_seq,
                    handle,
                    slot,
                    payload,
                })
            }
            Outbound::Response(r) => Frame::CmdResponse(r),
            Outbound::Event(e) => {
                if ws.send(Message::text(e)).is_err() {
                    return false;
                }
                continue;
            }
            Outbound::Close { .. } => return false,
        };
        if ws.send(Message::binary(msg.encode())).is_err() {
            return false;
        }
    }
    true
}

fn connection(tcp: TcpStream, hub: &HubHandle, stop: &AtomicBool) {
    let _ = tcp.set_nodelay(true);
    let Ok(raw) = tcp.try_clone() else { return };
    let Ok(mut ws) = tungstenite::accept_hdr(tcp, |req: &Request, mut resp: Response| {
        if req.headers().contains_key("Sec-WebSocket-Protocol") {
            resp.headers_mut()
                .insert("Sec-WebSocket-Protocol", SUB_PROTOCOL.parse().unwrap());
        }
        Ok(resp)
    }) else {
        return;
    };
    let _ = raw.set_read_timeout(Some(Duration::from_millis(2)));
    let session = hub.open_session(Role::Admin, None).unwrap();
    let mut poll_seq = 0;
    while !stop.load(Ordering::Relaxed) {
        let answer = match ws.read() {
            Ok(Message::Text(t)) => control(hub, &session, t.as_str()),
            Ok(Message::Binary(b)) => {
                if let Ok(Frame::CmdRequest(req)) = Frame::decode(&b) {
                    let done = Completion::to_outbox(req.tag, Arc::clone(session.outbox()));
                    hub.submit(&session, req, done);
                }
                None
            }
            Ok(_) => None,
            Err(WsError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(_) => break,
        };
        if let Some(a) = answer
            && ws.send(Message::text(a)).is_err()
        {
            break;
        }
        if !pump(&mut ws, &session, &mut poll_seq) {
            break;
        }
    }
    hub.close_session(session.id());
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the acceptor.
        let _ = TcpStream::connect(
            self.url
                .trim_start_matches("ws://")
                .trim_end_matches("/dspi/v1"),
        );
    }
}

fn serve(hub: &Hub) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/dspi/v1", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let handle = hub.handle();
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let Ok(tcp) = tcp else { return };
            let (hub, flag) = (handle.clone(), Arc::clone(&flag));
            std::thread::spawn(move || connection(tcp, &hub, &flag));
        }
    });
    Server { url, stop }
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
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (dev, remote, reads) = device("METER");
    hub.attach(Box::new(dev), 0).unwrap();
    let server = serve(&hub);
    remote.set_status(block(&remote, 0x4000));

    // One client: its feed, at the rate it asked for, while it asks far faster.
    let a = connect(&server.url);
    wait_until("the first client's meters", || first_peak(&a) > 0.0);
    let one = rate(&reads, &[&a], Duration::from_secs(2));

    // A second client: the same read, not another.
    let b = connect(&server.url);
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
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    let (dev, remote, _) = device("REPLUG");
    let handle = hub.attach(Box::new(dev), 0).unwrap();
    let server = serve(&hub);
    remote.set_status(block(&remote, 0x4000));
    let core = connect(&server.url);
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
