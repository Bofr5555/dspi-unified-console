//! A plain USB pipe to one DSPi, through the core's `usb` feature (nusb; IOKit
//! on macOS, `macos_iokit`): WP SW-M1.
//!
//! The Mac app's in-process Swift hub (`LinkHub`) owns the local device and
//! shares it: the app's own core is its session 1 (CG26's `HostTransport`) and
//! remote consoles are later sessions. The hub needs the device's control
//! transfers and notification packets unchanged, for every session, so it
//! cannot go through a [`crate::DspiCore`] (whose raw calls refuse the bulk
//! and voltage opcodes, and whose echo filter would take every session's
//! writes as its own). [`UsbPipe`] is that pipe: open by serial, control IN and
//! OUT on the vendor interface, and a blocking read of the notification
//! endpoint. [`usb_pipe_scan`] lists the DSPis on the bus without opening
//! any, for the app's hot-plug poll.
//!
//! It replaces the app's own IOKit driver (`USBDevice.swift`) and retires at
//! SW-M3, when the Rust hub (`dspi-hub`, which owns USB the same way) takes
//! the Swift hub's place. Not in the C ABI or the Android contract.
//!
//! Errors are [`DspiError`]s: [`DspiError::Disconnected`] when the device has
//! gone or the pipe is closed; [`DspiError::Refused`] with status 0 when the
//! device stalls (as [`crate::DspiCore::raw_get`] reports it);
//! [`DspiError::Link`] with code `timeout` when a transfer times out;
//! [`DspiError::Transfer`] for any other failure; [`DspiError::Unsupported`]
//! in a build without the `usb` feature (iOS, wasm) and on Android.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dspi_transport::{NotificationSource, Transport, TransportError};

use crate::records::DeviceInfo;
use crate::{DspiError, Result};

/// The longest notification wait one call may ask for: the caller's thread
/// must come back often enough to notice a close.
const MAX_NOTIFY_WAIT_MS: u32 = 2_000;

/// The DSPis on USB, by enumeration only: nothing is opened or claimed, so it
/// is cheap enough for a half-second poll and lists a device another program
/// holds like any other. Serial order. Empty without the `usb` feature, on
/// Android, and when enumeration fails (logged). Never fails.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn usb_pipe_scan() -> Vec<DeviceInfo> {
    crate::device::usb_scan_quiet()
}

/// One open DSPi on USB: its vendor interface claimed, nothing else. See the
/// module documentation. Every method may be called from any thread; control
/// transfers are serialised, and a notification read does not hold them up.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct UsbPipe {
    serial: String,
    /// The control path; `None` once closed.
    control: Mutex<Option<Box<dyn Transport>>>,
    /// The notification endpoint; `None` once closed or if the device has none.
    notes: Mutex<Option<Box<dyn NotificationSource>>>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pipe's error for a transport failure (module documentation).
fn pipe_error(e: TransportError) -> DspiError {
    match e {
        TransportError::Stalled { .. } => DspiError::Refused { status: 0 },
        TransportError::TimedOut { .. } => DspiError::Link {
            code: "timeout".into(),
            msg: e.to_string(),
        },
        TransportError::Disconnected
        | TransportError::NotFound
        | TransportError::SerialNotFound(_) => DspiError::Disconnected,
        other => DspiError::Transfer {
            msg: other.to_string(),
        },
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl UsbPipe {
    /// Open the DSPi with this serial and claim its vendor interface (only
    /// that one: the audio and MIDI interfaces stay with the system, so a DAW
    /// holding the DSPi's MIDI port does not keep it out). Blocks for the
    /// open: call it off the UI thread.
    ///
    /// Errors: [`DspiError::Disconnected`] if no DSPi with that serial is on
    /// the bus; [`DspiError::Transfer`] if it cannot be opened (held by
    /// another program, no permission); [`DspiError::Unsupported`] without
    /// the `usb` feature and on Android.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn open(serial: String) -> Result<Arc<Self>> {
        #[cfg(all(feature = "usb", not(target_os = "android")))]
        {
            let transport =
                dspi_transport::UsbTransport::open_serial(&serial).map_err(pipe_error)?;
            Ok(Arc::new(Self::over(Box::new(transport), serial)))
        }
        #[cfg(not(all(feature = "usb", not(target_os = "android"))))]
        {
            let _ = serial;
            Err(DspiError::Unsupported)
        }
    }

    /// The serial it was opened with.
    pub fn serial(&self) -> String {
        self.serial.clone()
    }

    /// One vendor IN transfer: `opcode`, `value` as `wValue`, at most `len`
    /// bytes back, as the device sent them (a short reply is not an error).
    /// Blocks for the transfer.
    pub fn control_in(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let mut control = guard(&self.control);
        let t = control.as_mut().ok_or(DspiError::Disconnected)?;
        t.control_in_upto(opcode, value, len).map_err(pipe_error)
    }

    /// One vendor OUT transfer: `opcode`, `value` as `wValue`, `data` as the
    /// payload. Success means the device took the transfer. Blocks for it.
    pub fn control_out(&self, opcode: u8, value: u16, data: Vec<u8>) -> Result<()> {
        let mut control = guard(&self.control);
        let t = control.as_mut().ok_or(DspiError::Disconnected)?;
        t.control_out(opcode, value, &data).map_err(pipe_error)
    }

    /// Wait up to `timeout_ms` (at most 2 s) for one notification packet, as
    /// the device sent it (keep-alives included). Empty when nothing came in
    /// time. Call it from one reader thread; it blocks that thread only.
    ///
    /// Errors: [`DspiError::Disconnected`] when the device has gone or the
    /// pipe is closed (the reader should stop), others as the module says.
    pub fn read_notification(&self, timeout_ms: u32) -> Result<Vec<u8>> {
        let wait = Duration::from_millis(u64::from(timeout_ms.min(MAX_NOTIFY_WAIT_MS)));
        let mut notes = guard(&self.notes);
        let source = notes.as_mut().ok_or(DspiError::Disconnected)?;
        match source.read(wait) {
            Ok(packet) => Ok(packet),
            Err(TransportError::TimedOut { .. }) => Ok(Vec::new()),
            Err(e) => Err(pipe_error(e)),
        }
    }

    /// Release the interface. Every later call fails with
    /// [`DspiError::Disconnected`]. Waits for a notification read in
    /// progress (at most its timeout). Idempotent.
    pub fn close(&self) {
        drop(guard(&self.control).take());
        drop(guard(&self.notes).take());
    }

    /// Whether [`UsbPipe::close`] has run.
    pub fn is_closed(&self) -> bool {
        guard(&self.control).is_none()
    }
}

impl UsbPipe {
    /// A pipe over any transport: the USB one, or a test double.
    #[cfg_attr(
        not(all(feature = "usb", not(target_os = "android"))),
        allow(dead_code)
    )]
    pub(crate) fn over(transport: Box<dyn Transport>, serial: String) -> Self {
        let notes = transport.notifications();
        Self {
            serial,
            control: Mutex::new(Some(transport)),
            notes: Mutex::new(notes),
        }
    }
}

#[cfg(test)]
mod tests_wp_swm1 {
    use super::*;
    use dspi_proto::generated::opcodes::{REQ_GET_PLATFORM, REQ_SET_MASTER_VOLUME};
    use dspi_transport::FakeDevice;

    fn pipe() -> UsbPipe {
        let fake = FakeDevice::demo();
        let serial = fake.descriptor().serial.clone();
        UsbPipe::over(Box::new(fake), serial)
    }

    #[test]
    fn wp_swm1_control_transfers_pass_through() {
        let p = pipe();
        let platform = p.control_in(REQ_GET_PLATFORM, 0, 7).expect("platform");
        assert!(!platform.is_empty() && platform.len() <= 7);
        p.control_out(REQ_SET_MASTER_VOLUME, 0, (-12.0f32).to_le_bytes().to_vec())
            .expect("write");
        assert!(!p.serial().is_empty());
    }

    #[test]
    fn wp_swm1_a_closed_pipe_is_disconnected_and_close_is_idempotent() {
        let p = pipe();
        assert!(!p.is_closed());
        p.close();
        p.close();
        assert!(p.is_closed());
        assert!(matches!(
            p.control_in(REQ_GET_PLATFORM, 0, 7),
            Err(DspiError::Disconnected)
        ));
        assert!(matches!(
            p.control_out(REQ_SET_MASTER_VOLUME, 0, vec![0; 4]),
            Err(DspiError::Disconnected)
        ));
        assert!(matches!(
            p.read_notification(10),
            Err(DspiError::Disconnected)
        ));
    }

    #[test]
    fn wp_swm1_notifications_arrive_and_a_quiet_wait_is_empty() {
        let p = pipe();
        // Drain whatever the fake queued at open, then expect quiet.
        for _ in 0..64 {
            if p.read_notification(5).expect("read").is_empty() {
                break;
            }
        }
        assert!(p.read_notification(5).expect("quiet").is_empty());
        // A write of ours makes the device notify.
        p.control_out(REQ_SET_MASTER_VOLUME, 0, (-6.0f32).to_le_bytes().to_vec())
            .expect("write");
        let mut got = Vec::new();
        for _ in 0..20 {
            got = p.read_notification(20).expect("read");
            if !got.is_empty() {
                break;
            }
        }
        assert!(!got.is_empty(), "the write's notification");
    }

    #[test]
    fn wp_swm1_errors_map_to_the_documented_kinds() {
        assert!(matches!(
            pipe_error(TransportError::Stalled { opcode: 1 }),
            DspiError::Refused { status: 0 }
        ));
        assert!(matches!(
            pipe_error(TransportError::TimedOut { opcode: 1, retries: 0 }),
            DspiError::Link { ref code, .. } if code == "timeout"
        ));
        assert!(matches!(
            pipe_error(TransportError::Disconnected),
            DspiError::Disconnected
        ));
        assert!(matches!(
            pipe_error(TransportError::SerialNotFound("x".into())),
            DspiError::Disconnected
        ));
        assert!(matches!(
            pipe_error(TransportError::Usb("busy".into())),
            DspiError::Transfer { .. }
        ));
    }

    #[test]
    fn wp_swm1_scan_never_fails_and_an_absent_serial_does_not_open() {
        // The container has no DSPi: the list is empty, or whatever is there.
        let _ = usb_pipe_scan();
        match UsbPipe::open("SWM1-NO-SUCH-SERIAL".into()) {
            Err(DspiError::Disconnected | DspiError::Unsupported | DspiError::Transfer { .. }) => {}
            Err(e) => panic!("unexpected error {e:?}"),
            Ok(_) => panic!("opened a device that is not there"),
        }
    }
}
