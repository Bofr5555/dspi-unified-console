//! A simulated DSPi: the demo device, and a stateful fixture for tests.
//!
//! Unlike [`crate::MockTransport`] (scripted replies, for the awkward paths),
//! this behaves like a device: it holds a V32 bulk image, answers the bulk
//! reads (single-shot and chunked) and the platform/version reads from it,
//! applies writes to the image, queues the `PARAM_CHANGED` notifications a
//! real device would send, and answers the extension channel (`EXT_LIST`,
//! the crossfeed pair map, the core-voltage status).
//!
//! Model it on the macOS Console's `FakeTransport` and the Android app's
//! `FakeDspiCore`. Owned by WP T4 (docs/core-merge/PLAN.md).

use std::time::Duration;

use crate::{
    DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps, TransportKind,
};

/// A captured device to start from: a `REQ_GET_PLATFORM` reply and a V32
/// bulk image. `tests/fixtures/link/device/` holds a real RP2350 capture.
#[derive(Debug, Clone)]
pub struct Capture {
    pub platform: Vec<u8>,
    pub bulk: Vec<u8>,
}

/// Which optional firmware features the fake answers for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeOptions {
    /// Answer the extension channel (the Bofr5555/DSPi fork's K340 build).
    pub extensions: bool,
}

pub struct FakeDevice {
    descriptor: DeviceDescriptor,
    #[allow(dead_code)]
    capture: Capture,
    #[allow(dead_code)]
    options: FakeOptions,
}

impl FakeDevice {
    pub fn new(capture: Capture, options: FakeOptions) -> Self {
        Self {
            descriptor: DeviceDescriptor {
                serial: "DEMO0000DSPI0001".into(),
                bus_id: "demo".into(),
                address: 0,
            },
            capture,
            options,
        }
    }

    /// The built-in demo device: the RP2350 capture bundled with this crate,
    /// with the extension channel on.
    pub fn demo() -> Self {
        todo!("WP T4: bundle the capture (include_bytes!) and build the demo")
    }
}

impl Transport for FakeDevice {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let _ = (opcode, value, len);
        todo!("WP T4")
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        let _ = (opcode, value, data);
        todo!("WP T4")
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        todo!("WP T4: notifications for the writes applied")
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Fake,
            chunked_bulk: true,
            remote: false,
        }
    }
}

#[allow(dead_code)]
const NOTIFY_POLL: Duration = Duration::from_millis(20);

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP T4"]
    fn the_demo_reads_back_its_capture() {
        todo!("WP T4")
    }

    #[test]
    #[ignore = "WP T4"]
    fn a_write_changes_the_image_and_notifies() {
        todo!("WP T4")
    }
}
