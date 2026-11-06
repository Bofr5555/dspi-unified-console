//! Devices: discovery, opening and closing, over every transport.
//!
//! Owned by WP C1 (docs/core-merge/PLAN.md); hotplug and reconnection are
//! WP C1b (`hotplug.rs`). Every `open_*` ends in
//! [`DspiCore::attach_probed`](crate::DspiCore), so the connect sequence is
//! the same on every transport.
//!
//! Thread rules (PLAN §1.2): never join a thread (the notification reader,
//! the RTA worker) while holding the `Inner` lock. A connection is taken out
//! under the lock and closed by [`shut`] with the guard dropped: its
//! notification reader is stopped and joined first, then the transport is
//! dropped. Stopping the reader before its device goes away means an
//! intentional close never reads as an unplug: the old reader's
//! `is_disconnected()` is never set by it, and the taken-out connection is
//! not reachable from the core any more anyway.
//!
//! `selected` (the serial C1b reconnects) is set only for USB devices
//! (`vid != 0`): a demo, MIDI or Link device is not reopened over USB.

use std::sync::{Arc, Mutex, PoisonError};

use dspi_proto::frame::DEFAULT_MFR_ID;
use dspi_transport::midi::{MidiInput, MidiPipe, MidiTransport};
use dspi_transport::{FakeDevice, Transport};

use crate::records::DeviceInfo;
use crate::{Connection, DspiCore, DspiError, Result, state};

/// The legacy USB vendor id of firmware before May 2026 (`USB_VID` is the
/// current one); the PID is the same.
pub(crate) const LEGACY_USB_VID: u16 = 0x2E8A;

/// Whether a USB device's ids name a DSPi (current or legacy VID).
pub(crate) fn is_dspi(vid: u16, pid: u16) -> bool {
    pid == dspi_proto::USB_PID && (vid == dspi_proto::USB_VID || vid == LEGACY_USB_VID)
}

/// Highest preamp (dB) an RP2040 applies correctly: its Q28 multiplier
/// overflows above +18.06 dB.
pub(crate) const PREAMP_MAX_DB_RP2040: f32 = 18.0;
/// Highest preamp (dB) an RP2350 applies.
pub(crate) const PREAMP_MAX_DB_RP2350: f32 = 24.0;

/// The receiving half of a MIDI connection, created before the device is
/// opened so the app's MIDI receive callback can feed it while
/// [`DspiCore::open_midi`] waits for the device's replies.
///
/// Usage: build it with [`MidiInputHandle::new`] over the app's send pipe,
/// hand every received byte to [`MidiInputHandle::receive`] (from the MIDI
/// callback, in any split), then call [`DspiCore::open_midi`] with it.
/// A handle opens one connection: after `open_midi` (successful or not),
/// build a new handle to open again. Dropping the handle does not close the
/// device ([`DspiCore::disconnect`] does).
pub struct MidiInputHandle {
    input: MidiInput,
    /// The sending half until `open_midi` takes it.
    transport: Mutex<Option<MidiTransport>>,
}

impl MidiInputHandle {
    /// A MIDI connection to a DSPi that sends through `pipe` (Android
    /// `MidiInputPort`, iOS CoreMIDI, ...), with the firmware's default
    /// manufacturer id. `serial` names the device for the picker. No I/O.
    pub fn new(pipe: Arc<dyn MidiPipe>, serial: String) -> Arc<Self> {
        let (transport, input) = MidiTransport::new(pipe, DEFAULT_MFR_ID, &serial);
        Arc::new(Self {
            input,
            transport: Mutex::new(Some(transport)),
        })
    }

    /// Feed received MIDI bytes (raw MIDI 1.0 stream, SysEx included), in
    /// any split, from any thread. Foreign data is ignored. Never blocks
    /// beyond a short lock.
    pub fn receive(&self, bytes: Vec<u8>) {
        self.input.receive(&bytes)
    }

    /// The platform reported the MIDI port closed or the device unplugged:
    /// the request in flight fails at once and later ones fail with
    /// [`DspiError::Disconnected`]. Idempotent.
    pub fn close(&self) {
        self.input.close()
    }

    /// The sending half, once.
    fn take_transport(&self) -> Option<MidiTransport> {
        self.transport
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Close a connection taken out of `Inner`, with the lock released: stop and
/// join the notification reader, then drop the transport (and with it the
/// session). In this order the reader never sees the drop as an unplug.
pub(crate) fn shut(conn: Connection) {
    let Connection {
        notifications,
        link,
        ..
    } = conn;
    drop(notifications);
    drop(link);
}

impl DspiCore {
    /// Take the open connection out (and reset the mirror to its defaults)
    /// under the lock, then close it with the lock released.
    pub(crate) fn close_connection(&self) {
        let old = {
            let mut inner = self.lock();
            inner.state = state::default_state();
            inner.conn.take()
        };
        if let Some(conn) = old {
            shut(conn);
        }
    }

    /// Adopt a USB device the platform already opened: Android's
    /// `UsbDeviceConnection.getFileDescriptor()`. The core `dup(2)`s the fd
    /// and owns the duplicate, so the app keeps (and later closes) its own;
    /// it claims the vendor interface 2 only, never the audio interfaces.
    /// Replaces any open device, then runs the connect sequence.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a negative or unusable
    /// `fd`; [`DspiError::Disconnected`] if it is not a DSPi; transfer
    /// errors from the connect sequence. Linux and Android only;
    /// [`DspiError::Unsupported`] elsewhere.
    pub fn open_fd(&self, fd: i32, serial: String, vid: u16) -> Result<()> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::BorrowedFd;
            if fd < 0 {
                return Err(DspiError::InvalidArgument {
                    msg: format!("file descriptor {fd} is negative"),
                });
            }
            // SAFETY: `fd` is non-negative and, by this method's contract,
            // an open descriptor the caller owns and keeps open for the
            // duration of the call. The borrow lives only for the `dup`
            // below; the core keeps the duplicate, never `fd` itself.
            let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
            let owned = borrowed
                .try_clone_to_owned()
                .map_err(|e| DspiError::InvalidArgument {
                    msg: format!("cannot duplicate file descriptor {fd}: {e}"),
                })?;
            // The same device may still be open through an older fd, whose
            // claim of interface 2 would make this one fail.
            self.close_connection();
            let transport = dspi_transport::UsbTransport::from_fd(owned, &serial)?;
            self.attach_probed(Box::new(transport), serial, vid)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = (fd, serial, vid);
            Err(DspiError::Unsupported)
        }
    }

    /// Close the device opened by [`DspiCore::open_fd`] (the same as
    /// [`DspiCore::disconnect`]); afterwards the app may close its
    /// `UsbDeviceConnection`. Not called `close` because UniFFI's Kotlin
    /// objects already have `close()` (free the object).
    pub fn close_fd(&self) {
        self.disconnect()
    }

    /// Enumerate DSPis on USB (nusb): VID `2E8B` or the legacy VID, PID
    /// `FEAA`, with a readable serial, in serial order. Updates
    /// [`DspiCore::inaccessible_devices`]. Empty on Android, where apps
    /// cannot enumerate (use `open_fd`). Never fails: an enumeration error
    /// gives an empty list.
    pub fn scan_devices(&self) -> Vec<DeviceInfo> {
        // The device we hold is listed without being opened a second time.
        let ours = {
            let inner = self.lock();
            inner
                .conn
                .as_ref()
                .filter(|c| c.vid != 0)
                .map(|c| c.serial.clone())
        };
        let (devices, inaccessible) = usb::scan(ours.as_deref());
        let mut inner = self.lock();
        inner.devices = devices.clone();
        inner.inaccessible = inaccessible;
        devices
    }

    /// Highest preamp (dB) the connected chip applies correctly: +18 on an
    /// RP2040 (its Q28 multiplier overflows above), +24 on an RP2350. +18
    /// while nothing is connected (the mirror's default platform).
    pub fn preamp_max_db(&self) -> f32 {
        if self.lock().state.platform_id == 1 {
            PREAMP_MAX_DB_RP2350
        } else {
            PREAMP_MAX_DB_RP2040
        }
    }

    /// DSPis the last scan saw but could not open: no permission (Linux udev
    /// rule), no WinUSB binding, or held by another program.
    pub fn inaccessible_devices(&self) -> u32 {
        self.lock().inaccessible
    }

    /// Open the USB DSPi with this serial and run the connect sequence,
    /// replacing any open device. The serial is remembered for reconnection
    /// (WP C1b) even if the open fails, so a device that arrives later is
    /// picked up.
    ///
    /// Errors: [`DspiError::Disconnected`] if no DSPi with that serial is on
    /// the bus; transfer errors (no permission, busy) from opening it or from
    /// the connect sequence. [`DspiError::Unsupported`] on Android (use
    /// [`DspiCore::open_fd`]).
    pub fn select_device(&self, serial: String) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            let _ = serial;
            Err(DspiError::Unsupported)
        }
        #[cfg(not(target_os = "android"))]
        {
            self.lock().selected = Some(serial.clone());
            // Close first: the device may be the one we hold, and a second
            // claim of its interface 2 would fail.
            self.close_connection();
            let vid = usb::vendor_of(&serial).ok_or(DspiError::Disconnected)?;
            let transport = dspi_transport::UsbTransport::open_serial(&serial)?;
            self.attach_probed(Box::new(transport), serial, vid)
        }
    }

    /// Close the open device, whatever its transport, and reset the mirror
    /// to its defaults. The selected serial is kept, so a replugged device
    /// reconnects (WP C1b). Blocks until the notification reader has stopped
    /// (at most about a quarter of a second).
    pub fn disconnect(&self) {
        self.close_connection()
    }

    /// Whether a device is open.
    pub fn is_connected(&self) -> bool {
        self.lock().conn.is_some()
    }

    /// Open a device over a transport the embedder built (a Link device, a
    /// test double, a MIDI transport from `dspi_transport::midi::midir_port`)
    /// and run the connect sequence. Replaces any open device. `vid` is 0 for
    /// non-USB transports. Errors as for the connect sequence.
    pub fn open_transport(
        &self,
        transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        self.attach_probed(transport, serial, vid)
    }

    /// Open the built-in demo device (`dspi_transport::FakeDevice::demo`):
    /// a simulated RP2350 that answers reads and writes and notifies.
    /// Replaces any open device.
    pub fn open_demo(&self) -> Result<()> {
        let demo = FakeDevice::demo();
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial, 0)
    }

    /// Open a DSPi through the USB MIDI control channel (Android
    /// `MidiManager`, iOS CoreMIDI): the connect sequence over `input`'s
    /// pipe, while the app's MIDI callback feeds `input`. Replaces any open
    /// device.
    ///
    /// Errors: [`DspiError::InvalidArgument`] if `input` was already used to
    /// open (build a new [`MidiInputHandle`]); otherwise as for the connect
    /// sequence (a silent device is a timeout).
    pub fn open_midi(&self, input: Arc<MidiInputHandle>) -> Result<()> {
        let transport = input
            .take_transport()
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "this MIDI handle has already been opened; create a new one".into(),
            })?;
        let serial = transport.descriptor().serial.clone();
        self.attach_probed(Box::new(transport), serial, 0)
    }
}

/// USB enumeration through nusb.
mod usb {
    use crate::records::DeviceInfo;

    /// Every DSPi on the bus, and how many could not be opened. `ours`
    /// (the device the core holds) is listed without opening it.
    #[cfg(target_os = "android")]
    pub(super) fn scan(ours: Option<&str>) -> (Vec<DeviceInfo>, u32) {
        let _ = ours;
        (Vec::new(), 0)
    }

    /// Every DSPi on the bus, and how many could not be opened. `ours`
    /// (the device the core holds) is listed without opening it.
    #[cfg(not(target_os = "android"))]
    pub(super) fn scan(ours: Option<&str>) -> (Vec<DeviceInfo>, u32) {
        use nusb::MaybeFuture;

        let devices = match nusb::list_devices().wait() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("USB enumeration failed: {e}");
                return (Vec::new(), 0);
            }
        };
        let mut found = Vec::new();
        let mut inaccessible: u32 = 0;
        for d in devices.filter(|d| super::is_dspi(d.vendor_id(), d.product_id())) {
            let serial = d.serial_number().unwrap_or_default().to_string();
            let held = !serial.is_empty() && ours == Some(serial.as_str());
            if !held {
                match accessible(&d) {
                    Access::Yes => {}
                    Access::No => {
                        inaccessible = inaccessible.saturating_add(1);
                        continue;
                    }
                    Access::Gone => continue,
                }
            }
            // Identity is what selection and reconnection are keyed on: a
            // device without a serial is skipped, not guessed at.
            if serial.is_empty() {
                continue;
            }
            found.push(DeviceInfo {
                serial,
                location_id: location_id(&d),
            });
        }
        found.sort_by(|a, b| a.serial.cmp(&b.serial));
        (found, inaccessible)
    }

    /// The vendor id the DSPi with this serial is on the bus under.
    #[cfg(not(target_os = "android"))]
    pub(super) fn vendor_of(serial: &str) -> Option<u16> {
        use nusb::MaybeFuture;
        nusb::list_devices()
            .wait()
            .ok()?
            .filter(|d| super::is_dspi(d.vendor_id(), d.product_id()))
            .find(|d| d.serial_number() == Some(serial))
            .map(|d| d.vendor_id())
    }

    #[cfg(not(target_os = "android"))]
    enum Access {
        Yes,
        /// On the bus but not ours to open.
        No,
        /// Gone, or failed for another reason: not listed.
        Gone,
    }

    /// Whether this user can open the device. On Windows the vendor
    /// interface is claimed too (and released at once): that is where a
    /// missing WinUSB binding or another program's claim shows.
    #[cfg(not(target_os = "android"))]
    fn accessible(d: &nusb::DeviceInfo) -> Access {
        use nusb::{ErrorKind, MaybeFuture};
        let classify = |e: nusb::Error| match e.kind() {
            ErrorKind::PermissionDenied | ErrorKind::Busy => Access::No,
            #[cfg(windows)]
            ErrorKind::Unsupported | ErrorKind::NotFound => Access::No,
            _ => Access::Gone,
        };
        match d.open().wait() {
            #[cfg(windows)]
            Ok(dev) => match dev.claim_interface(super::VENDOR_IFACE).wait() {
                Ok(_) => Access::Yes,
                Err(e) => classify(e),
            },
            #[cfg(not(windows))]
            Ok(_) => Access::Yes,
            Err(e) => classify(e),
        }
    }

    /// The old core's `location_id`: IOKit's on macOS, else
    /// `address | bus << 8` (bus 0 when the platform's bus id is not a
    /// number).
    #[cfg(not(target_os = "android"))]
    fn location_id(d: &nusb::DeviceInfo) -> u32 {
        #[cfg(target_os = "macos")]
        {
            d.location_id()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let bus = d.bus_id().parse::<u8>().unwrap_or(0);
            u32::from(d.device_address()) | (u32::from(bus) << 8)
        }
    }
}

/// The vendor interface number, as `claim_interface` takes it.
#[cfg(windows)]
const VENDOR_IFACE: u8 = {
    assert!(dspi_proto::VENDOR_INTERFACE <= u8::MAX as u16);
    dspi_proto::VENDOR_INTERFACE as u8
};

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c1 {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, Weak};
    use std::time::{Duration, Instant};

    use dspi_proto::frame::{DEFAULT_MFR_ID, Frame, SysexDecoder, encode_sysex, status};
    use dspi_proto::generated::notify as n;
    use dspi_transport::midi::{MidiPipe, PipeError};
    use dspi_transport::{
        DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportError,
    };

    use super::*;
    use crate::NotifyListener;
    use crate::consts::COMPAT_OK;

    /// A transport around a `FakeDevice` whose notification source reports
    /// when it is dropped (its reader thread ended) and whether it ever saw
    /// the device as gone.
    struct Watched {
        dev: FakeDevice,
        flags: Arc<Flags>,
    }

    #[derive(Default)]
    struct Flags {
        source_dropped: AtomicBool,
        saw_disconnect: AtomicBool,
        sources_taken: AtomicUsize,
    }

    struct WatchedSource {
        inner: Box<dyn NotificationSource>,
        flags: Arc<Flags>,
    }

    impl NotificationSource for WatchedSource {
        fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
            let r = self.inner.read(timeout);
            if matches!(r, Err(TransportError::Disconnected)) {
                self.flags.saw_disconnect.store(true, Ordering::SeqCst);
            }
            r
        }
    }

    impl Drop for WatchedSource {
        fn drop(&mut self) {
            self.flags.source_dropped.store(true, Ordering::SeqCst);
        }
    }

    impl Transport for Watched {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> dspi_transport::Result<()> {
            self.dev.control_out(o, v, d)
        }
        fn max_transfer(&self) -> usize {
            self.dev.max_transfer()
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.dev.descriptor()
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            self.flags.sources_taken.fetch_add(1, Ordering::SeqCst);
            let inner = self.dev.notifications()?;
            Some(Box::new(WatchedSource {
                inner,
                flags: Arc::clone(&self.flags),
            }))
        }
    }

    fn watched() -> (Watched, Arc<Flags>, FakeRemote) {
        let dev = FakeDevice::demo();
        let remote = dev.remote();
        let flags = Arc::new(Flags::default());
        (
            Watched {
                dev,
                flags: Arc::clone(&flags),
            },
            flags,
            remote,
        )
    }

    /// A v2 `PARAM_CHANGED` for one byte at `offset` (the fake renumbers it).
    fn param_changed(offset: u16) -> Vec<u8> {
        let mut p = vec![2, n::NOTIFY_EVT_PARAM_CHANGED as u8, 0, 0];
        p.extend(offset.to_le_bytes());
        p.extend(1u16.to_le_bytes());
        p.extend([n::PARAM_SRC_HOST_SET as u8, 0, 0, 0, 1]);
        p
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn wp_c1_open_demo_connects_and_decodes_the_mirror() {
        let core = DspiCore::new();
        assert!(!core.is_connected());
        core.open_demo().unwrap();
        assert!(core.is_connected());
        let mut inner = core.lock();
        assert_eq!(inner.state.bulk_raw.len(), 6136);
        assert!(inner.state.bulk_valid);
        assert_eq!(inner.state.compat, COMPAT_OK);
        assert_eq!(inner.state.platform_id, 1, "the demo is an RP2350");
        assert!(inner.state.num_output_channels > 0);
        // No USB device: nothing to reconnect.
        assert_eq!(inner.selected, None);
        let conn = inner.conn().unwrap();
        assert!(conn.writable());
        assert!(conn.notifications.is_some());
        assert!(!conn.ext.supported.is_empty(), "the demo has extensions");
        assert!(conn.session().is_ok());
    }

    #[test]
    fn wp_c1_preamp_max_follows_the_chip() {
        let core = DspiCore::new();
        assert_eq!(core.preamp_max_db(), 18.0);
        core.open_demo().unwrap();
        assert_eq!(core.preamp_max_db(), 24.0);
        core.disconnect();
        assert_eq!(core.preamp_max_db(), 18.0);
    }

    #[test]
    fn wp_c1_disconnect_closes_and_resets_but_keeps_the_selection() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.lock().selected = Some("SER".into());
        core.disconnect();
        assert!(!core.is_connected());
        {
            let inner = core.lock();
            assert!(!inner.state.bulk_valid);
            assert_eq!(inner.selected.as_deref(), Some("SER"));
        }
        core.close_fd(); // already closed: a no-op
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1_reopening_joins_the_old_reader_before_dropping_its_device() {
        let core = DspiCore::new();
        let (first, flags, _remote) = watched();
        core.open_transport(Box::new(first), "ONE".into(), 0)
            .unwrap();
        assert_eq!(flags.sources_taken.load(Ordering::SeqCst), 1);
        assert!(!flags.source_dropped.load(Ordering::SeqCst));

        let (second, flags2, _) = watched();
        core.open_transport(Box::new(second), "TWO".into(), 0)
            .unwrap();
        // The old reader thread has ended (it owned the source) by the time
        // the new device is installed...
        assert!(flags.source_dropped.load(Ordering::SeqCst));
        // ...and it was stopped before its device was dropped, so it never
        // saw the intentional close as an unplug.
        assert!(!flags.saw_disconnect.load(Ordering::SeqCst));
        assert_eq!(flags2.sources_taken.load(Ordering::SeqCst), 1);
        assert_eq!(core.lock().conn().unwrap().serial, "TWO");

        core.disconnect();
        assert!(flags2.source_dropped.load(Ordering::SeqCst));
        assert!(!flags2.saw_disconnect.load(Ordering::SeqCst));
    }

    #[test]
    fn wp_c1_a_failed_connect_leaves_no_reader_and_takes_the_source_once() {
        let core = DspiCore::new();
        let (dev, flags, remote) = watched();
        remote.disconnect(); // unplugged before the probe
        let err = core.open_transport(Box::new(dev), "GONE".into(), 0);
        assert_eq!(err, Err(DspiError::Disconnected));
        assert!(!core.is_connected());
        assert_eq!(flags.sources_taken.load(Ordering::SeqCst), 1);
        assert!(flags.source_dropped.load(Ordering::SeqCst), "reader joined");
    }

    #[test]
    fn wp_c1_a_failed_connect_closes_the_previous_device() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let (dev, _, remote) = watched();
        remote.disconnect();
        assert!(
            core.open_transport(Box::new(dev), "GONE".into(), 0)
                .is_err()
        );
        assert!(!core.is_connected());
        assert!(!core.lock().state.bulk_valid, "no stale mirror");
    }

    /// Wakes by calling back into the core, as an app's handler might.
    struct Reentrant {
        core: Mutex<Weak<DspiCore>>,
        calls: AtomicUsize,
    }

    impl NotifyListener for Reentrant {
        fn on_notifications(&self) {
            let core = self.core.lock().unwrap().upgrade();
            if let Some(core) = core {
                let _ = core.is_connected();
                let _ = core.preamp_max_db();
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn wp_c1_a_listener_calling_back_into_the_core_does_not_deadlock() {
        let core = DspiCore::new();
        let listener = Arc::new(Reentrant {
            core: Mutex::new(Arc::downgrade(&core)),
            calls: AtomicUsize::new(0),
        });
        core.listeners.notify.set(Some(listener.clone()));
        let (dev, _, remote) = watched();
        core.open_transport(Box::new(dev), "ONE".into(), 0).unwrap();
        remote.push(param_changed(4000));
        wait_for("the listener", || listener.calls.load(Ordering::SeqCst) > 0);
        // And a reopen while the listener is active still completes.
        remote.push(param_changed(4001));
        core.open_demo().unwrap();
        assert!(core.is_connected());
    }

    #[test]
    fn wp_c1_the_echo_filter_is_new_on_reopen() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.lock().conn().unwrap().echo.bulk_write_started();
        let pending = |core: &DspiCore| {
            core.lock()
                .conn()
                .unwrap()
                .echo
                .own_bulk_pending(Instant::now())
        };
        assert!(pending(&core));
        core.open_demo().unwrap();
        assert!(!pending(&core));
    }

    #[test]
    fn wp_c1_scan_finds_nothing_without_usb() {
        // The build containers have no USB bus.
        let core = DspiCore::new();
        assert!(core.scan_devices().is_empty());
        assert_eq!(core.inaccessible_devices(), 0);
        assert!(core.lock().devices.is_empty());
    }

    #[test]
    fn wp_c1_selecting_an_absent_device_fails_but_is_remembered() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let r = core.select_device("NOPE".into());
        if cfg!(target_os = "android") {
            assert_eq!(r, Err(DspiError::Unsupported));
        } else {
            assert_eq!(r, Err(DspiError::Disconnected));
            assert_eq!(core.lock().selected.as_deref(), Some("NOPE"));
            assert!(!core.is_connected());
        }
    }

    #[test]
    fn wp_c1_is_dspi_accepts_both_vendor_ids() {
        assert!(is_dspi(0x2E8B, 0xFEAA));
        assert!(is_dspi(0x2E8A, 0xFEAA));
        assert!(!is_dspi(0x2E8B, 0x0003));
        assert!(!is_dspi(0x2E8A, 0x000F));
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn wp_c1_open_fd_refuses_bad_descriptors() {
        use std::os::fd::AsRawFd;
        let core = DspiCore::new();
        assert!(matches!(
            core.open_fd(-1, "X".into(), 0x2E8B),
            Err(DspiError::InvalidArgument { .. })
        ));
        // A descriptor that is open but not a usbfs device: refused, and the
        // caller's own descriptor stays open (the core used a duplicate).
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = file.as_raw_fd();
        assert!(core.open_fd(fd, "X".into(), 0x2E8B).is_err());
        assert!(!core.is_connected());
        // SAFETY: `file` owns `fd` and is alive.
        let again = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }.try_clone_to_owned();
        assert!(again.is_ok(), "the caller's fd is still open");
    }

    /// A DSPi on the far side of a MIDI pipe: decodes SysEx requests,
    /// answers them from a `FakeDevice` and feeds the replies back through
    /// the handle, as the app's MIDI callback would.
    struct MidiBridge {
        dev: Mutex<FakeDevice>,
        decoder: Mutex<SysexDecoder>,
        input: Mutex<Option<Arc<MidiInputHandle>>>,
    }

    impl MidiBridge {
        fn answer(&self, frame: Frame) -> Frame {
            let mut dev = self.dev.lock().unwrap();
            match frame {
                Frame::Get {
                    request,
                    value,
                    length,
                    ..
                } => match dev.control_in_upto(request, value, length) {
                    Ok(payload) => Frame::GetResponse {
                        status: status::OK,
                        payload,
                    },
                    Err(_) => Frame::GetResponse {
                        status: status::ERROR,
                        payload: Vec::new(),
                    },
                },
                Frame::Set {
                    request,
                    value,
                    payload,
                    ..
                } => Frame::SetResponse {
                    status: match dev.control_out(request, value, &payload) {
                        Ok(()) => status::OK,
                        Err(_) => status::ERROR,
                    },
                },
                _ => Frame::SetResponse {
                    status: status::ERROR,
                },
            }
        }
    }

    impl MidiPipe for MidiBridge {
        fn send(&self, sysex: &[u8]) -> std::result::Result<(), PipeError> {
            let messages = self.decoder.lock().unwrap().feed(sysex);
            let input = self.input.lock().unwrap().clone().unwrap();
            for m in messages {
                let reply = self.answer(m.frame.unwrap());
                let bytes = encode_sysex(DEFAULT_MFR_ID, m.tag, &reply);
                // Split, as a platform's MIDI callback may deliver it.
                for c in bytes.chunks(61) {
                    input.receive(c.to_vec());
                }
            }
            Ok(())
        }
    }

    fn midi_bridge() -> (Arc<MidiBridge>, Arc<MidiInputHandle>) {
        let bridge = Arc::new(MidiBridge {
            dev: Mutex::new(FakeDevice::demo()),
            decoder: Mutex::new(SysexDecoder::new(DEFAULT_MFR_ID)),
            input: Mutex::new(None),
        });
        let handle = MidiInputHandle::new(bridge.clone(), "MIDI-DEMO".into());
        *bridge.input.lock().unwrap() = Some(handle.clone());
        (bridge, handle)
    }

    #[test]
    fn wp_c1_open_midi_runs_the_connect_sequence_over_the_pipe() {
        let core = DspiCore::new();
        let (_bridge, handle) = midi_bridge();
        core.open_midi(handle.clone()).unwrap();
        {
            let mut inner = core.lock();
            assert!(inner.state.bulk_valid);
            assert_eq!(inner.state.bulk_raw.len(), 6136);
            let conn = inner.conn().unwrap();
            assert_eq!(conn.serial, "MIDI-DEMO");
            assert_eq!(conn.vid, 0);
            assert_eq!(conn.caps.kind, dspi_transport::TransportKind::Midi);
            assert!(conn.notifications.is_some());
        }
        // A handle opens once.
        assert!(matches!(
            core.open_midi(handle),
            Err(DspiError::InvalidArgument { .. })
        ));
        // ...and that refusal did not close the open device.
        assert!(core.is_connected());
        core.disconnect();
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1_open_midi_fails_cleanly_when_the_port_is_closed() {
        let core = DspiCore::new();
        let (_bridge, handle) = midi_bridge();
        handle.close();
        assert!(core.open_midi(handle).is_err());
        assert!(!core.is_connected());
    }
}
