//! WP HB0: a connect through the hub costs what a direct one does.
//!
//! The core's connect probes stall on purpose ("this build lacks it"). A hub that ran
//! each stall through the busy window would add 600 ms a stall, about 5 s a connect,
//! and hold the device's one queue meanwhile. One test in its own binary: cargo runs
//! test binaries one at a time, so no parallel test can hide or cause the delay.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::DspiCore;
use dspi_hub::app::AppLink;
use dspi_hub::{Hub, HubConfig};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps, TransportError,
};

/// Well under one retried stall per probe, and well over a loaded CI's direct connect.
const BOUND: Duration = Duration::from_secs(2);

type Counts = Arc<Mutex<BTreeMap<u8, usize>>>;

/// The demo device, counting the requests that stall, by opcode.
struct Counting(FakeDevice, Counts);

impl Counting {
    fn note<T>(&self, opcode: u8, r: dspi_transport::Result<T>) -> dspi_transport::Result<T> {
        if matches!(r, Err(TransportError::Stalled { .. })) {
            *self.1.lock().unwrap().entry(opcode).or_default() += 1;
        }
        r
    }
}

impl Transport for Counting {
    fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        let r = self.0.control_in(o, v, l);
        self.note(o, r)
    }
    fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
        let r = self.0.control_in_upto(o, v, l);
        self.note(o, r)
    }
    fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
        let r = self.0.control_out(o, v, d);
        self.note(o, r)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.0.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.0.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.0.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.0.caps()
    }
}

#[test]
fn wp_hb0_a_connect_through_the_hub_is_not_slowed_by_stalled_probes() {
    let counts = Counts::default();
    let dev = Counting(FakeDevice::demo(), Arc::clone(&counts));
    let serial = dev.descriptor().serial.clone();
    let hub = Hub::start(HubConfig::in_process("test")).unwrap();
    hub.attach(Box::new(dev), 0).unwrap();
    let core = DspiCore::new();
    let _link = AppLink::attach(&hub.handle(), Arc::clone(&core)).unwrap();

    let start = Instant::now();
    core.select_device(serial).unwrap();
    core.fetch_all().unwrap();
    let took = start.elapsed();

    let counts = counts.lock().unwrap();
    println!("connect {took:?}, stalls by opcode {counts:02X?}");
    assert!(
        !counts.is_empty(),
        "the demo must stall something to prove this"
    );
    assert!(took < BOUND, "{took:?}; stalls by opcode {counts:02X?}");
}
