//! WP QD5: the demo device's output-latency timing, which a client's test uses to
//! reach the firmware's deferred apply, and its refusal elsewhere.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dspi_transport::{DeviceDescriptor, FakeDevice, Transport, TransportCaps};

use super::*;
use crate::ext::tests_wp_c8::k340;

fn field(status: &OutputLatency, name: &str) -> String {
    status
        .fields
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.value.clone())
        .unwrap_or_default()
}

#[test]
fn wp_qd5_a_deferred_request_shows_the_old_id_and_pending_then_applies() {
    let p = k340();
    let before = p.core.fetch_output_latency().unwrap();
    assert_eq!(field(&before, "pending"), "0");
    p.core.demo_latency_timing(1, 2, 0).unwrap();
    p.core.set_output_latency(3).unwrap();
    assert_eq!(p.core.demo_latency_sets().unwrap(), 1);

    for _ in 0..2 {
        let s = p.core.fetch_output_latency().unwrap();
        assert_eq!(field(&s, "pending"), "1");
        assert_eq!(field(&s, "requested_id"), field(&before, "requested_id"));
    }
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(field(&s, "pending"), "0");
    assert_eq!(field(&s, "requested_id"), "3");
    assert_eq!(p.core.demo_latency_sets().unwrap(), 1, "reads send nothing");
}

#[test]
fn wp_qd5_a_held_request_stays_pending_until_released() {
    let p = k340();
    p.core.demo_latency_timing(2, 0, 0).unwrap();
    p.core.set_output_latency(3).unwrap();
    for _ in 0..5 {
        let s = p.core.fetch_output_latency().unwrap();
        assert_eq!(field(&s, "pending"), "1");
    }
    p.core.demo_latency_timing(0, 0, 0).unwrap();
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(field(&s, "pending"), "0");
    assert_eq!(field(&s, "requested_id"), "3");
}

#[test]
fn wp_qd5_a_dropped_request_keeps_the_old_id() {
    let p = k340();
    let before = p.core.fetch_output_latency().unwrap();
    p.core.demo_latency_timing(3, 0, 0).unwrap();
    p.core.set_output_latency(3).unwrap();
    let s = p.core.fetch_output_latency().unwrap();
    assert_eq!(field(&s, "pending"), "0");
    assert_eq!(field(&s, "requested_id"), field(&before, "requested_id"));
}

#[test]
fn wp_qd5_a_bad_mode_and_no_device_are_refused() {
    let p = k340();
    assert!(matches!(
        p.core.demo_latency_timing(4, 0, 0),
        Err(DspiError::InvalidArgument { .. })
    ));
    let core = DspiCore::new();
    assert_eq!(
        core.demo_latency_timing(0, 0, 0),
        Err(DspiError::Disconnected)
    );
    assert_eq!(core.demo_latency_sets(), Err(DspiError::Disconnected));
}

/// A simulated device that reports itself as a real transport, counting traffic.
struct RealKind {
    dev: FakeDevice,
    kind: TransportKind,
    calls: Arc<AtomicUsize>,
}

impl Transport for RealKind {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.dev.control_in(opcode, value, len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.dev.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: self.kind,
            ..self.dev.caps()
        }
    }
}

#[test]
fn wp_qd5_a_real_transport_kind_refuses_with_no_traffic() {
    for kind in [TransportKind::Usb, TransportKind::Midi, TransportKind::Link] {
        let calls = Arc::<AtomicUsize>::default();
        let core = DspiCore::new();
        core.attach_probed(
            Box::new(RealKind {
                dev: FakeDevice::demo(),
                kind,
                calls: calls.clone(),
            }),
            "QD5".into(),
            0,
        )
        .unwrap();
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(
            core.demo_latency_timing(2, 0, 0),
            Err(DspiError::Unsupported)
        );
        assert_eq!(core.demo_latency_sets(), Err(DspiError::Unsupported));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before,
            "{kind:?} sent something"
        );
    }
}
