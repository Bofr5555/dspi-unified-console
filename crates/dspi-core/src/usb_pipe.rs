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
//! any, and [`usb_pipe_wait_change`] waits for nusb's hot-plug events, so the
//! app's hot-plug follows the bus at once and polls only as a fallback.
//!
//! The first scan starts one process-wide hot-plug watcher (`nusb::watch_devices`,
//! a thread of its own). Besides the events, it keeps nusb's macOS event thread
//! (`nusb-events`) alive for the process: without a registration of its own, every
//! open spawned that thread again and waited for it, which Xcode's thread
//! checker reported as a priority inversion (fixup round, review swm1).
//!
//! It replaces the app's own IOKit driver (`USBDevice.swift`) and retires at
//! SW-M3, when the Rust hub (`dspi-hub`, which owns USB the same way) takes
//! the Swift hub's place. Not in the C ABI or the Android contract.
//!
//! Errors are [`DspiError`]s: [`DspiError::Disconnected`] when the device has
//! gone or the pipe is shut down; [`DspiError::Refused`] with status 0 when the
//! device stalls (as [`crate::DspiCore::raw_get`] reports it);
//! [`DspiError::Link`] with code `timeout` when a transfer times out, and with
//! code `busy` when the vendor interface is held by another program (macOS:
//! the claim is refused for exclusive access);
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
    watch::start();
    crate::device::usb_scan_quiet()
}

/// Wait until the bus has changed since `seen` (a device arrived or left), or
/// `timeout_ms` (at most 2 s) has passed, and return the change count to pass
/// next time. Start with 0. Blocks the calling thread only. Without a hot-plug
/// watcher (no `usb` feature, Android, or the platform refused one: logged) it
/// just waits out the timeout and returns `seen`, so the caller's scan after it
/// is a plain poll. Never fails.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn usb_pipe_wait_change(seen: u64, timeout_ms: u32) -> u64 {
    watch::wait(
        seen,
        Duration::from_millis(u64::from(timeout_ms.min(MAX_NOTIFY_WAIT_MS))),
    )
}

/// The process-wide hot-plug watcher: one thread polling nusb's
/// `watch_devices` stream, counting its events.
mod watch {
    use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
    use std::time::Duration;

    #[derive(Default)]
    pub(super) struct Shared {
        /// Events seen so far.
        changes: Mutex<u64>,
        wake: Condvar,
    }

    /// `None` once a start was tried and no watcher runs.
    static WATCH: OnceLock<Option<&'static Shared>> = OnceLock::new();

    /// Start the watcher once (no-op afterwards).
    pub(super) fn start() {
        WATCH.get_or_init(spawn);
    }

    #[cfg(all(feature = "usb", any(target_os = "macos", target_os = "linux")))]
    fn spawn() -> Option<&'static Shared> {
        let watch = match nusb::watch_devices() {
            Ok(w) => w,
            Err(e) => {
                log::warn!("USB hot-plug events unavailable, polling only: {e}");
                return None;
            }
        };
        let shared: &'static Shared = Box::leak(Box::default());
        let spawned = std::thread::Builder::new()
            .name("dspi-usb-hotplug".into())
            .spawn(move || run(watch, shared));
        match spawned {
            Ok(_) => Some(shared),
            Err(e) => {
                log::warn!("USB hot-plug thread not started, polling only: {e}");
                None
            }
        }
    }

    #[cfg(not(all(feature = "usb", any(target_os = "macos", target_os = "linux"))))]
    fn spawn() -> Option<&'static Shared> {
        None
    }

    /// Poll the stream for the life of the process: park until its waker
    /// unparks this thread, count each event.
    #[cfg(all(feature = "usb", any(target_os = "macos", target_os = "linux")))]
    fn run(mut watch: nusb::hotplug::HotplugWatch, shared: &'static Shared) {
        use futures_core::Stream;
        use std::pin::Pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        loop {
            match Pin::new(&mut watch).poll_next(&mut cx) {
                Poll::Ready(Some(_)) => shared.bump(),
                Poll::Ready(None) => {
                    log::warn!("USB hot-plug events ended, polling only");
                    return;
                }
                Poll::Pending => std::thread::park(),
            }
        }
    }

    impl Shared {
        #[cfg_attr(
            not(all(feature = "usb", any(target_os = "macos", target_os = "linux"))),
            allow(dead_code)
        )]
        fn bump(&self) {
            let mut n = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
            *n = n.wrapping_add(1);
            self.wake.notify_all();
        }
    }

    pub(super) fn wait(seen: u64, timeout: Duration) -> u64 {
        start();
        let Some(Some(shared)) = WATCH.get() else {
            dspi_transport::clock::sleep(timeout); // -- WP SW-M2 -- the time seam (wasm-core)
            return seen;
        };
        let n = shared
            .changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (n, _) = shared
            .wake
            .wait_timeout_while(n, timeout, |n| *n == seen)
            .unwrap_or_else(PoisonError::into_inner);
        *n
    }

    #[cfg(test)]
    pub(super) fn bump_for_test(shared: &Shared) {
        shared.bump();
    }

    #[cfg(test)]
    pub(super) fn wait_on(shared: &Shared, seen: u64, timeout: Duration) -> u64 {
        let n = shared
            .changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (n, _) = shared
            .wake
            .wait_timeout_while(n, timeout, |n| *n == seen)
            .unwrap_or_else(PoisonError::into_inner);
        *n
    }
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
        // -- fixup F4 -- On macOS a refused claim is another program holding the
        // vendor interface (nusb: "could not open interface for exclusive
        // access"; dspi-transport maps it to PermissionDenied). Elsewhere it is
        // a real permission problem and stays a transfer error with its help text.
        #[cfg(target_os = "macos")]
        TransportError::PermissionDenied => DspiError::Link {
            code: "busy".into(),
            msg: e.to_string(),
        },
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
            // -- WP PGWIRE -- the generation probe first, on the raw wire
            let transport = dspi_transport::paged::open(Box::new(transport)).map_err(pipe_error)?;
            Ok(Arc::new(Self::over(transport, serial)))
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
    /// progress (at most its timeout). Idempotent. (Not `close`: UniFFI's
    /// Kotlin objects already have `AutoCloseable.close()`.)
    pub fn shutdown(&self) {
        drop(guard(&self.control).take());
        drop(guard(&self.notes).take());
    }

    /// Whether [`UsbPipe::shutdown`] has run.
    pub fn is_shut_down(&self) -> bool {
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
    fn wp_swm1_a_shut_down_pipe_is_disconnected_and_shutdown_is_idempotent() {
        let p = pipe();
        assert!(!p.is_shut_down());
        p.shutdown();
        p.shutdown();
        assert!(p.is_shut_down());
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
    fn wp_swm1_a_held_interface_is_busy_on_macos() {
        let e = pipe_error(TransportError::PermissionDenied);
        if cfg!(target_os = "macos") {
            assert!(
                matches!(e, DspiError::Link { ref code, .. } if code == "busy"),
                "{e:?}"
            );
        } else {
            assert!(matches!(e, DspiError::Transfer { .. }), "{e:?}");
        }
    }

    #[test]
    fn wp_swm1_wait_change_wakes_on_an_event_and_times_out_without_one() {
        let shared = watch::Shared::default();
        let t = std::time::Instant::now();
        assert_eq!(watch::wait_on(&shared, 0, Duration::from_millis(30)), 0);
        assert!(t.elapsed() >= Duration::from_millis(25));
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                watch::bump_for_test(&shared);
            });
            let t = std::time::Instant::now();
            assert_eq!(watch::wait_on(&shared, 0, Duration::from_secs(5)), 1);
            assert!(t.elapsed() < Duration::from_secs(4), "woken by the event");
        });
        // A change already counted returns at once.
        assert_eq!(watch::wait_on(&shared, 0, Duration::from_secs(5)), 1);
        // The public call never fails, watcher or not.
        let _ = usb_pipe_wait_change(0, 10);
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
