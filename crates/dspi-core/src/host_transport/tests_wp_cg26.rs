//! WP CG26: a host-supplied transport, scripted over the fake device.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_proto::generated::{self as g, notify as nfy, opcodes as op};
use dspi_transport::FakeDevice;

use super::*;
use crate::consts::{NOTIFY_DISCONNECTED, NOTIFY_STATE};

const WAIT: Duration = Duration::from_secs(8);
const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One transfer as it went out.
#[derive(Debug, Clone, PartialEq)]
enum Call {
    In {
        opcode: u8,
        value: u16,
        len: u16,
    },
    Out {
        opcode: u8,
        value: u16,
        data: Vec<u8>,
    },
    Lock {
        reason: String,
        timeout_ms: u32,
    },
    Unlock,
}

type Log = Arc<Mutex<Vec<Call>>>;

/// What the scripted host does instead of answering.
#[derive(Debug, Clone)]
enum Fault {
    Fail(DspiError),
    /// Take this long, then answer.
    Hang(Duration),
    /// Answer one byte more than asked.
    Long,
    /// Answer one byte less than asked.
    Short,
}

/// A host transport over a fake device, as Swift's `FakeTransport` would be:
/// it records every transfer and fails on request.
struct ScriptedHost {
    dev: Mutex<FakeDevice>,
    kind: HostTransportKind,
    log: Log,
    fault: Mutex<Option<Fault>>,
    /// What `lock` answers instead of `Ok`.
    lock_error: Mutex<Option<DspiError>>,
    calls: Mutex<usize>,
}

impl ScriptedHost {
    fn new(dev: FakeDevice, kind: HostTransportKind) -> Arc<Self> {
        Arc::new(Self {
            dev: Mutex::new(dev),
            kind,
            log: Log::default(),
            fault: Mutex::new(None),
            lock_error: Mutex::new(None),
            calls: Mutex::new(0),
        })
    }

    fn set_fault(&self, f: Option<Fault>) {
        *self.fault.lock().unwrap() = f;
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }

    /// The fault, if it replaces the answer; a hang sleeps and then answers.
    fn fault(&self) -> Option<Fault> {
        *self.calls.lock().unwrap() += 1;
        let f = self.fault.lock().unwrap().clone();
        if let Some(Fault::Hang(d)) = f {
            std::thread::sleep(d);
            return None;
        }
        f
    }
}

impl HostTransport for ScriptedHost {
    fn control_in(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let fault = self.fault();
        if let Some(Fault::Fail(e)) = fault {
            return Err(e);
        }
        let ask = match fault {
            Some(Fault::Long) => len + 1,
            _ => len,
        };
        self.log
            .lock()
            .unwrap()
            .push(Call::In { opcode, value, len });
        let mut reply = self
            .dev
            .lock()
            .unwrap()
            .control_in_upto(opcode, value, ask)?;
        match fault {
            Some(Fault::Long) => reply.resize(usize::from(len) + 1, 0),
            Some(Fault::Short) => {
                reply.pop();
            }
            _ => {}
        }
        Ok(reply)
    }

    fn control_out(&self, opcode: u8, value: u16, data: Vec<u8>) -> Result<()> {
        if let Some(Fault::Fail(e)) = self.fault() {
            return Err(e);
        }
        self.log.lock().unwrap().push(Call::Out {
            opcode,
            value,
            data: data.clone(),
        });
        Ok(self.dev.lock().unwrap().control_out(opcode, value, &data)?)
    }

    fn kind(&self) -> HostTransportKind {
        self.kind
    }

    fn lock(&self, reason: String, timeout_ms: u32) -> Result<()> {
        if let Some(Fault::Fail(e)) = self.fault() {
            return Err(e);
        }
        if let Some(e) = self.lock_error.lock().unwrap().clone() {
            return Err(e);
        }
        self.log
            .lock()
            .unwrap()
            .push(Call::Lock { reason, timeout_ms });
        Ok(())
    }

    fn unlock(&self) {
        *self.calls.lock().unwrap() += 1;
        self.log.lock().unwrap().push(Call::Unlock);
    }
}

/// The fake device as the core's own transport, recording the same way.
struct Recorder {
    dev: FakeDevice,
    log: Log,
}

impl Transport for Recorder {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.log
            .lock()
            .unwrap()
            .push(Call::In { opcode, value, len });
        self.dev.control_in(opcode, value, len)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.log.lock().unwrap().push(Call::In {
            opcode,
            value,
            len: max_len,
        });
        self.dev.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        self.log.lock().unwrap().push(Call::Out {
            opcode,
            value,
            data: data.to_vec(),
        });
        self.dev.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> TResult<()> {
        self.log.lock().unwrap().push(Call::Lock {
            reason: reason.to_owned(),
            timeout_ms: timeout.as_millis() as u32,
        });
        Ok(())
    }

    fn unlock(&mut self) {
        self.log.lock().unwrap().push(Call::Unlock);
    }
}

/// Forwards the fake device's notification stream to `host_notify`, as the
/// app's endpoint callback would, until dropped.
struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Pump {
    fn start(core: &Arc<DspiCore>, dev: &FakeDevice, origin: Option<u16>) -> Self {
        let mut source = dev.notifications().unwrap();
        let core = Arc::downgrade(core);
        let stop = Arc::new(AtomicBool::new(false));
        let halt = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !halt.load(Ordering::Relaxed) {
                let Ok(a) = source.read_attributed(Duration::from_millis(20)) else {
                    return;
                };
                if let Some(core) = core.upgrade() {
                    core.host_notify(a.packet, origin);
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn opened(
    kind: HostTransportKind,
) -> (Arc<DspiCore>, Arc<ScriptedHost>, dspi_transport::FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let host = ScriptedHost::new(dev, kind);
    let core = DspiCore::new();
    core.open_host(host.clone(), "HOST-1".into()).unwrap();
    (core, host, remote)
}

#[test]
fn wp_cg26_the_mirror_is_the_fake_devices() {
    let direct = DspiCore::new();
    direct.open_demo().unwrap();
    direct.fetch_all().unwrap();
    let want = direct.get_state();
    for kind in [
        HostTransportKind::Usb,
        HostTransportKind::Fake,
        HostTransportKind::Link { session: 1 },
    ] {
        let (core, _host, _) = opened(kind);
        assert!(core.is_connected());
        core.fetch_all().unwrap();
        assert_eq!(core.get_state(), want, "{kind:?}");
    }
}

#[test]
fn wp_cg26_the_bytes_and_their_order_are_the_core_transports() {
    let ops = |core: &DspiCore| {
        core.fetch_all().unwrap();
        core.set_master_volume(-12.5).unwrap();
        core.set_preamp(3.0).unwrap();
        core.set_loudness(true).unwrap();
        core.set_master_volume(-4.0).unwrap();
    };
    let direct_log = Log::default();
    let direct = DspiCore::new();
    direct
        .open_transport(
            Box::new(Recorder {
                dev: FakeDevice::demo(),
                log: Arc::clone(&direct_log),
            }),
            "HOST-1".into(),
            0,
        )
        .unwrap();
    ops(&direct);

    let (core, host, _) = opened(HostTransportKind::Usb);
    ops(&core);
    let got = host.log.lock().unwrap().clone();
    let want = direct_log.lock().unwrap().clone();
    assert_eq!(got, want);
    // The writes went out as issued.
    let outs: Vec<(u8, Vec<u8>)> = got
        .iter()
        .filter_map(|c| match c {
            Call::Out { opcode, data, .. } if *opcode == op::REQ_SET_MASTER_VOLUME => {
                Some((*opcode, data.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        outs,
        vec![
            (op::REQ_SET_MASTER_VOLUME, (-12.5f32).to_le_bytes().to_vec()),
            (op::REQ_SET_MASTER_VOLUME, (-4.0f32).to_le_bytes().to_vec()),
        ]
    );
    assert_eq!(core.get_state(), direct.get_state());
}

#[test]
fn wp_cg26_notifications_reach_the_mirror() {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    let _pump = Pump::start(&core, &dev, None);
    core.open_host(
        ScriptedHost::new(dev, HostTransportKind::Usb),
        "HOST-1".into(),
    )
    .unwrap();
    core.notify_tap_set(true);

    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-9.0f32).to_le_bytes(), GPIO));
    wait_until("the knob's change", || {
        core.process_notifications();
        core.get_state().master_volume_db == -9.0
    });
    let tapped = core.notify_tap_drain();
    assert!(tapped.iter().any(|n| !n.own), "{tapped:?}");

    // Our own write's echo is ours.
    core.set_master_volume(-20.0).unwrap();
    let mut ours = Vec::new();
    wait_until("our echo", || {
        core.process_notifications();
        ours.extend(core.notify_tap_drain());
        ours.iter()
            .any(|n| n.packet.get(1) == Some(&(nfy::NOTIFY_EVT_PARAM_CHANGED as u8)))
    });
    let changed: Vec<_> = ours
        .iter()
        .filter(|n| n.packet.get(1) == Some(&(nfy::NOTIFY_EVT_PARAM_CHANGED as u8)))
        .collect();
    assert!(changed.iter().all(|n| n.own), "{ours:?}");
}

#[test]
fn wp_cg26_a_link_host_attributes_by_origin() {
    let (core, _host, _) = opened(HostTransportKind::Link { session: 1 });
    core.notify_tap_set(true);
    let mut p = vec![
        2,
        nfy::NOTIFY_EVT_PARAM_CHANGED as u8,
        0,
        0,
        0,
        0,
        4,
        0,
        0,
        0,
        0,
        0,
    ];
    p[4..6].copy_from_slice(&(g::OFF_MASTER_VOLUME as u16).to_le_bytes());
    p[8] = nfy::PARAM_SRC_HOST_SET as u8;
    p.extend_from_slice(&(-7.0f32).to_le_bytes());
    core.host_notify(p.clone(), Some(7));
    let mut got = Vec::new();
    wait_until("the other session's change", || {
        let r = core.process_notifications();
        got.extend(core.notify_tap_drain());
        r.flags & NOTIFY_STATE != 0
    });
    assert_eq!(core.get_state().master_volume_db, -7.0);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].origin, Some(7));
    assert!(!got[0].own);

    p[3] = 1;
    core.host_notify(p, Some(1));
    let mut got = Vec::new();
    wait_until("our session's echo", || {
        core.process_notifications();
        got.extend(core.notify_tap_drain());
        !got.is_empty()
    });
    assert_eq!(got[0].origin, Some(1));
    assert!(got[0].own, "session 1 is us");
}

#[test]
fn wp_cg26_host_errors_are_typed_and_a_lost_host_ends_the_connection() {
    let (core, host, _) = opened(HostTransportKind::Usb);

    host.set_fault(Some(Fault::Fail(DspiError::Transfer {
        msg: "IOUSBHost timeout".into(),
    })));
    match core.set_master_volume(-3.0) {
        Err(DspiError::Transfer { msg }) => assert!(msg.contains("IOUSBHost timeout"), "{msg}"),
        other => panic!("{other:?}"),
    }
    host.set_fault(Some(Fault::Long));
    assert!(matches!(core.fetch_all(), Err(DspiError::Transfer { .. })));
    host.set_fault(Some(Fault::Fail(DspiError::Locked)));
    assert_eq!(core.set_master_volume(-3.0), Err(DspiError::Locked));
    // Nothing of that ends the connection.
    host.set_fault(None);
    core.set_master_volume(-3.0).unwrap();
    assert_eq!(core.process_notifications().flags & NOTIFY_DISCONNECTED, 0);

    host.set_fault(Some(Fault::Fail(DspiError::Disconnected)));
    assert_eq!(core.set_master_volume(-2.0), Err(DspiError::Disconnected));
    host.set_fault(None);
    let calls = host.calls();
    assert_eq!(core.set_master_volume(-2.0), Err(DspiError::Disconnected));
    assert_eq!(host.calls(), calls, "a gone host is not called again");
    wait_until("NOTIFY_DISCONNECTED", || {
        core.process_notifications().flags & NOTIFY_DISCONNECTED != 0
    });
}

#[test]
fn wp_cg26_a_hung_host_ends_disconnected() {
    let dev = FakeDevice::demo();
    let host = ScriptedHost::new(dev, HostTransportKind::Usb);
    let core = DspiCore::new();
    core.open_host_with(host.clone(), "HOST-1".into(), Duration::from_millis(300))
        .unwrap();
    host.set_fault(Some(Fault::Hang(Duration::from_millis(500))));
    assert_eq!(core.set_master_volume(-2.0), Err(DspiError::Disconnected));
    assert_eq!(core.fetch_all(), Err(DspiError::Disconnected));
    wait_until("NOTIFY_DISCONNECTED", || {
        core.process_notifications().flags & NOTIFY_DISCONNECTED != 0
    });
}

#[test]
fn wp_cg26_a_host_failing_the_connect_leaves_nothing_connected() {
    for fault in [
        Fault::Fail(DspiError::Transfer { msg: "x".into() }),
        Fault::Fail(DspiError::Disconnected),
        Fault::Fail(DspiError::Unsupported),
        Fault::Hang(Duration::from_millis(500)),
        Fault::Long,
    ] {
        let host = ScriptedHost::new(FakeDevice::demo(), HostTransportKind::Usb);
        host.set_fault(Some(fault.clone()));
        let core = DspiCore::new();
        let r = core.open_host_with(host, "HOST-1".into(), Duration::from_millis(300));
        assert!(
            matches!(r, Err(DspiError::Transfer { .. } | DspiError::Disconnected)),
            "{fault:?}: {r:?}"
        );
        assert!(!core.is_connected(), "{fault:?}");
    }
}

#[test]
fn wp_cg26_notify_without_a_host_connection_is_ignored() {
    let core = DspiCore::new();
    core.host_notify(vec![2, 0, 0, 0], None);
    core.host_notify(Vec::new(), Some(1));
    assert_eq!(core.process_notifications().flags, 0);

    // A demo device replacing the host connection is not fed.
    let (core, _host, _) = opened(HostTransportKind::Usb);
    core.open_demo().unwrap();
    core.notify_tap_set(true);
    let mut p = vec![2, nfy::NOTIFY_EVT_PRESET_LOADED as u8, 0, 0, 3, 0, 0, 0];
    core.host_notify(p.clone(), None);
    p[3] = 1;
    core.host_notify(p, None);
    std::thread::sleep(Duration::from_millis(100));
    core.process_notifications();
    assert!(core.notify_tap_drain().is_empty());
}

#[test]
fn wp_cg26_reply_lengths_are_checked() {
    let host = ScriptedHost::new(FakeDevice::demo(), HostTransportKind::Usb);
    let (mut t, _feed) = Adapter::new(host.clone(), "HOST-1".into(), HANG_LIMIT);
    let platform = op::REQ_GET_PLATFORM;
    let full = t.control_in_upto(platform, 0, 7).unwrap();
    assert!(!full.is_empty());

    host.set_fault(Some(Fault::Short));
    let wanted = full.len() as u16;
    assert!(matches!(
        t.control_in(platform, 0, wanted),
        Err(TransportError::ShortRead { opcode, .. }) if opcode == platform
    ));
    assert_eq!(
        t.control_in_upto(platform, 0, wanted).unwrap().len(),
        full.len() - 1,
        "a short reply is the answer where the size is the device's"
    );
    host.set_fault(Some(Fault::Long));
    assert!(matches!(
        t.control_in_upto(platform, 0, wanted),
        Err(TransportError::Malformed(_))
    ));
    host.set_fault(Some(Fault::Fail(DspiError::Refused { status: 2 })));
    assert!(matches!(
        t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]),
        Err(TransportError::Stalled { opcode }) if opcode == op::REQ_SET_MASTER_VOLUME
    ));
}

// -- review c4i finding 2: the hub's device lock (WP CG24) reaches the host.

/// The lock calls in the host's log since `from`.
fn lock_calls(host: &ScriptedHost, from: usize) -> Vec<Call> {
    host.log.lock().unwrap()[from..]
        .iter()
        .filter(|c| matches!(c, Call::Lock { .. } | Call::Unlock))
        .cloned()
        .collect()
}

#[test]
fn wp_cg26_the_device_lock_reaches_the_host_and_flows_nest_in_it() {
    let (core, host, _) = opened(HostTransportKind::Link { session: 1 });
    let from = host.log.lock().unwrap().len();
    core.device_lock_acquire("restore".into(), 5_000).unwrap();
    // The core's own locked flow nests: the hub is not asked again.
    core.save_preset_tok(2).unwrap();
    core.device_lock_release();
    assert_eq!(
        lock_calls(&host, from),
        vec![
            Call::Lock {
                reason: "restore".into(),
                timeout_ms: 5_000
            },
            Call::Unlock
        ]
    );

    // A flow of its own takes and gives back the lock around its writes.
    let from = host.log.lock().unwrap().len();
    core.save_preset_tok(3).unwrap();
    let flow = host.log.lock().unwrap()[from..].to_vec();
    assert!(matches!(flow.first(), Some(Call::Lock { .. })), "{flow:?}");
    assert_eq!(flow.last(), Some(&Call::Unlock), "{flow:?}");
}

#[test]
fn wp_cg26_a_lock_held_by_another_session_refuses_the_flow() {
    let (core, host, _) = opened(HostTransportKind::Link { session: 1 });
    *host.lock_error.lock().unwrap() = Some(DspiError::Locked);
    assert_eq!(
        core.device_lock_acquire("x".into(), 1_000),
        Err(DspiError::Locked)
    );
    let from = host.log.lock().unwrap().len();
    assert_eq!(core.save_preset_tok(2), Err(DspiError::Locked));
    let after = host.log.lock().unwrap()[from..].to_vec();
    assert!(
        !after
            .iter()
            .any(|c| matches!(c, Call::Out { .. } | Call::Unlock)),
        "nothing written and nothing to give back: {after:?}"
    );

    // Refusals of the lock are not stalls.
    *host.lock_error.lock().unwrap() = Some(DspiError::Unsupported);
    assert!(matches!(
        core.device_lock_acquire("x".into(), 1_000),
        Err(DspiError::Transfer { .. })
    ));
    *host.lock_error.lock().unwrap() = Some(DspiError::Denied);
    assert_eq!(
        core.device_lock_acquire("x".into(), 1_000),
        Err(DspiError::Denied)
    );
}

#[test]
fn wp_cg26_closing_with_the_lock_held_gives_it_back() {
    let (core, host, _) = opened(HostTransportKind::Link { session: 1 });
    core.device_lock_acquire("x".into(), 5_000).unwrap();
    let from = host.log.lock().unwrap().len();
    core.disconnect();
    assert_eq!(lock_calls(&host, from), vec![Call::Unlock]);

    // A host that went away is not called again, not even to unlock.
    let (core, host, _) = opened(HostTransportKind::Link { session: 1 });
    core.device_lock_acquire("x".into(), 5_000).unwrap();
    host.set_fault(Some(Fault::Fail(DspiError::Disconnected)));
    assert_eq!(core.set_master_volume(-2.0), Err(DspiError::Disconnected));
    let from = host.log.lock().unwrap().len();
    core.disconnect();
    assert_eq!(lock_calls(&host, from), vec![]);
}
