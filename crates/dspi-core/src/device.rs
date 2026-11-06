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
use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::packets::PlatformInfo;
use dspi_transport::midi::{MidiInput, MidiPipe, MidiTransport, PipeError};
use dspi_transport::{FakeDevice, Transport, TransportError};

use crate::callbacks::MidiSender;
use crate::consts::{COMPAT_FIRMWARE_TOO_NEW, COMPAT_FIRMWARE_TOO_OLD};
use crate::records::{DeviceInfo, DspState};
use crate::{Connection, DspiCore, DspiError, Result, rta, state};

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

/// A [`MidiPipe`] over the app's [`MidiSender`].
struct SenderPipe(Arc<dyn MidiSender>);

impl MidiPipe for SenderPipe {
    fn send(&self, sysex: &[u8]) -> std::result::Result<(), PipeError> {
        self.0.send_sysex(sysex.to_vec()).map_err(|e| match e {
            DspiError::Disconnected => PipeError::Closed,
            other => PipeError::Failed(other.to_string()),
        })
    }
}

/// A MIDI port to a DSPi: the app's sending pipe and the receiving side its
/// MIDI callback feeds. Create one per port, for the port's life.
///
/// Usage: build it with [`MidiInputHandle::new`] over the app's send pipe,
/// register a MIDI callback that hands every received byte to
/// [`MidiInputHandle::receive`] (in any split), then call
/// [`DspiCore::open_midi`] with it, as often as needed: each open starts a
/// fresh connection over the same pipe, and `receive` always feeds the
/// newest. The handle exists before the open because the callback must
/// deliver the device's replies while `open_midi` is still probing.
/// Dropping the handle does not close the device ([`DspiCore::disconnect`]
/// does).
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct MidiInputHandle {
    pipe: Arc<dyn MidiPipe>,
    serial: String,
    /// The receiving half of the newest connection over this port.
    current: Mutex<Option<MidiInput>>,
}

impl MidiInputHandle {
    /// A MIDI port to a DSPi that sends through `pipe` (Android
    /// `MidiInputPort`, iOS CoreMIDI, ...), with the firmware's default
    /// manufacturer id. `serial` names the device for the picker. No I/O.
    pub fn new(pipe: Arc<dyn MidiPipe>, serial: String) -> Arc<Self> {
        Arc::new(Self {
            pipe,
            serial,
            current: Mutex::new(None),
        })
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl MidiInputHandle {
    /// A MIDI port that sends through `sender`, the binding's form of
    /// [`MidiInputHandle::new`]. No I/O.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn with_sender(sender: Arc<dyn MidiSender>, serial: String) -> Arc<Self> {
        Self::new(Arc::new(SenderPipe(sender)), serial)
    }

    /// Feed received MIDI bytes (raw MIDI 1.0 stream, SysEx included), in
    /// any split, from any thread, to the newest connection. Foreign data,
    /// and anything before the first `open_midi`, is ignored. Never blocks
    /// beyond a short lock.
    pub fn receive(&self, bytes: Vec<u8>) {
        let input = self.lock().clone();
        if let Some(input) = input {
            input.receive(&bytes)
        }
    }

    /// The platform reported the MIDI port closed or the device unplugged:
    /// the current connection's request in flight fails at once and later
    /// ones fail with [`DspiError::Disconnected`]. Idempotent. A later
    /// `open_midi` starts afresh.
    ///
    /// Not called `close`: UniFFI's Kotlin objects already have a `close()`,
    /// which frees the object.
    pub fn close_port(&self) {
        let input = self.lock().clone();
        if let Some(input) = input {
            input.close()
        }
    }
}

impl MidiInputHandle {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<MidiInput>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new connection over the pipe; `receive` feeds it from now on (the
    /// previous one's input is closed: it is being replaced).
    fn connect(&self) -> MidiTransport {
        let (transport, input) =
            MidiTransport::new(Arc::clone(&self.pipe), DEFAULT_MFR_ID, &self.serial);
        let previous = self.lock().replace(input);
        if let Some(previous) = previous {
            previous.close();
        }
        transport
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

/// The read-only mirror for a device whose image cannot be mapped (PLAN
/// §1.5, "Unmappable images"), or `None` when `err` is not such a case or the
/// device does not identify itself (then `err` stands).
///
/// It applies to [`TransportError::Incompatible`] (pre-V16, an implausible
/// newer image) and, for a legacy-VID unit, to any read error but
/// `Disconnected`. The mirror holds what `GET_PLATFORM` says
/// (`state::apply_platform`), `bulk_valid` false, and `compat` too new when
/// the image's version byte is above this build's, too old otherwise
/// (including when the header cannot be read).
pub(crate) fn unmappable_mirror(
    t: &mut dyn Transport,
    err: &TransportError,
    vid: u16,
) -> Option<DspState> {
    let applies = match err {
        TransportError::Disconnected => false,
        TransportError::Incompatible(_) => true,
        _ => vid == LEGACY_USB_VID,
    };
    if !applies {
        return None;
    }
    let reply = t
        .control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_REQUEST_LEN)
        .ok()?;
    let mut mirror = state::default_state();
    state::apply_platform(&reply, &mut mirror).ok()?;
    let version = header_version(t);
    if let Some(v) = version {
        mirror.format_version = v;
    }
    mirror.compat = match version {
        Some(v) if u16::from(v) > WIRE_FORMAT_VERSION => COMPAT_FIRMWARE_TOO_NEW,
        _ => COMPAT_FIRMWARE_TOO_OLD,
    };
    mirror.bulk_valid = false;
    log::warn!("{err}; connected read-only");
    Some(mirror)
}

/// Read `GET_PLATFORM` into `mirror` (platform, output count, firmware
/// version): `Ok(true)` when applied. A lost device is
/// [`DspiError::Disconnected`]; any other failure (the probe has just read
/// the same request, so it is unlikely) is logged and gives `Ok(false)`, the
/// mirror untouched.
pub(crate) fn read_platform_into(t: &mut dyn Transport, mirror: &mut DspState) -> Result<bool> {
    let kind = t.caps().kind;
    let reply = dspi_transport::with_busy_retry(
        kind,
        || t.control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_REQUEST_LEN),
        op::REQ_GET_PLATFORM,
    );
    match reply {
        Ok(reply) => match state::apply_platform(&reply, mirror) {
            Ok(()) => Ok(true),
            Err(e) => {
                log::warn!("GET_PLATFORM reply not applied: {e}");
                Ok(false)
            }
        },
        Err(TransportError::Disconnected) => Err(DspiError::Disconnected),
        Err(e) => {
            log::warn!("GET_PLATFORM failed after the probe: {e}");
            Ok(false)
        }
    }
}

/// `wLength` of the `GET_PLATFORM` read: 7 bytes, the longest reply.
const PLATFORM_REQUEST_LEN: u16 = {
    assert!(PlatformInfo::REQUEST_LEN <= u16::MAX as usize);
    PlatformInfo::REQUEST_LEN as u16
};

/// The image's wire-format byte (header byte 0), from a short first read:
/// chunked where the transport has it, else single-shot (the firmware
/// answers `min(wLength, image)`). `None` if it cannot be read.
fn header_version(t: &mut dyn Transport) -> Option<u8> {
    let opcode = if t.caps().chunked_bulk {
        op::REQ_GET_ALL_PARAMS_CHUNK
    } else {
        op::REQ_GET_ALL_PARAMS
    };
    t.control_in_upto(opcode, 0, 8).ok()?.first().copied()
}

impl DspiCore {
    /// Take the open connection out (and reset the mirror to its defaults)
    /// under the lock, then stop its RTA worker and close it with the lock
    /// released.
    pub(crate) fn close_connection(&self) {
        let (old, worker) = {
            let mut inner = self.lock();
            inner.state = state::default_state();
            inner.cs = crate::cs::CsState::default();
            let worker = rta::take_worker(&mut inner);
            (inner.conn.take(), worker)
        };
        if let Some(worker) = worker {
            worker.stop();
        }
        if let Some(conn) = old {
            shut(conn);
        }
    }

    /// The last step of the connect sequence: install `conn` and `mirror`,
    /// then, with the lock released, stop whatever another thread installed
    /// meanwhile (the last open wins) and wake the [`crate::NotifyListener`]
    /// once, for packets that arrived during the connect.
    pub(crate) fn install(&self, conn: Connection, mirror: DspState, serial: String, vid: u16) {
        let reader = conn.notifications.is_some();
        let (displaced, worker) = {
            let mut inner = self.lock();
            inner.state = mirror;
            inner.selected = (vid != 0).then_some(serial);
            inner.cs = crate::cs::CsState::default();
            let worker = rta::take_worker(&mut inner);
            (inner.conn.replace(conn), worker)
        };
        if let Some(worker) = worker {
            worker.stop();
        }
        if let Some(old) = displaced {
            shut(old);
        }
        if reader {
            self.listeners.notify.call(|l| l.on_notifications());
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Adopt a USB device the platform already opened: Android's
    /// `UsbDeviceConnection.getFileDescriptor()`. The core `dup(2)`s the fd
    /// and owns the duplicate, so the app keeps (and later closes) its own;
    /// it claims the vendor interface 2 only, never the audio interfaces.
    /// Replaces any open device, then runs the connect sequence; on failure
    /// nothing is connected (the previous device is closed either way).
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

    /// Whether a device is open: `conn.is_some()`. It stays true after an
    /// unplug until the connection is closed (by C1b's hotplug poll, C7's
    /// notification handling, or [`DspiCore::disconnect`]); a call on it
    /// then fails with [`DspiError::Disconnected`].
    pub fn is_connected(&self) -> bool {
        self.lock().conn.is_some()
    }
}

impl DspiCore {
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
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Open the built-in demo device (`dspi_transport::FakeDevice::demo`):
    /// a simulated RP2350 that answers reads and writes and notifies.
    /// Replaces any open device.
    pub fn open_demo(&self) -> Result<()> {
        let demo = FakeDevice::demo();
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial, 0)
    }

    /// Open a DSPi through the USB MIDI control channel (Android
    /// `MidiManager`, iOS CoreMIDI): a fresh connection over `input`'s pipe,
    /// then the connect sequence, while the app's MIDI callback feeds
    /// `input`. Replaces any open device. The same handle may be opened
    /// again (a reconnect over the same port). Errors as for the connect
    /// sequence (a silent device is a timeout).
    pub fn open_midi(&self, input: Arc<MidiInputHandle>) -> Result<()> {
        let transport = input.connect();
        let serial = input.serial.clone();
        self.attach_probed(Box::new(transport), serial, 0)
    }
}

impl DspiCore {
    /// The DSPis on USB, by enumeration only: nothing is opened or claimed,
    /// so a device another program holds is listed like any other and
    /// `inaccessible_devices` is not updated. For C1b's hotplug diffs, which
    /// run every half second; [`DspiCore::scan_devices`] is the picker's
    /// scan. Does not touch `Inner`. Empty on Android.
    pub(crate) fn scan_quiet(&self) -> Vec<DeviceInfo> {
        usb::scan_quiet()
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
    /// (the device the core holds) is listed without opening it. A device
    /// without a serial is skipped before any open: identity is what
    /// selection and reconnection are keyed on.
    #[cfg(not(target_os = "android"))]
    pub(super) fn scan(ours: Option<&str>) -> (Vec<DeviceInfo>, u32) {
        let mut found = Vec::new();
        let mut inaccessible: u32 = 0;
        for d in dspis() {
            let Some(serial) = serial_of(&d) else {
                continue;
            };
            if ours != Some(serial.as_str()) && !accessible(&d) {
                inaccessible = inaccessible.saturating_add(1);
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

    /// Every DSPi with a serial on the bus, by enumeration only (nothing is
    /// opened), in serial order.
    #[cfg(not(target_os = "android"))]
    pub(super) fn scan_quiet() -> Vec<DeviceInfo> {
        let mut found: Vec<DeviceInfo> = dspis()
            .filter_map(|d| {
                Some(DeviceInfo {
                    serial: serial_of(&d)?,
                    location_id: location_id(&d),
                })
            })
            .collect();
        found.sort_by(|a, b| a.serial.cmp(&b.serial));
        found
    }

    /// Nothing to enumerate on Android.
    #[cfg(target_os = "android")]
    pub(super) fn scan_quiet() -> Vec<DeviceInfo> {
        Vec::new()
    }

    /// The DSPis on the bus (empty, logged, if enumeration fails).
    #[cfg(not(target_os = "android"))]
    fn dspis() -> impl Iterator<Item = nusb::DeviceInfo> {
        use nusb::MaybeFuture;
        let devices = match nusb::list_devices().wait() {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("USB enumeration failed: {e}");
                None
            }
        };
        devices
            .into_iter()
            .flatten()
            .filter(|d| super::is_dspi(d.vendor_id(), d.product_id()))
    }

    /// The device's serial, if it has a non-empty one.
    #[cfg(not(target_os = "android"))]
    fn serial_of(d: &nusb::DeviceInfo) -> Option<String> {
        d.serial_number()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
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

    /// Whether this user can open the device. On Windows the vendor
    /// interface is claimed too (and released at once): that is where a
    /// missing WinUSB binding or another program's claim shows. Any failure
    /// (permission, busy, no driver, or another error) counts: the device is
    /// on the bus but cannot be used.
    #[cfg(not(target_os = "android"))]
    fn accessible(d: &nusb::DeviceInfo) -> bool {
        use nusb::MaybeFuture;
        match d.open().wait() {
            #[cfg(windows)]
            Ok(dev) => dev.claim_interface(super::VENDOR_IFACE).wait().is_ok(),
            #[cfg(not(windows))]
            Ok(_) => true,
            Err(e) => {
                log::debug!("a DSPi cannot be opened: {e}");
                false
            }
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
    use crate::consts::{COMPAT_FIRMWARE_TOO_NEW, COMPAT_FIRMWARE_TOO_OLD, COMPAT_OK};

    /// A transport around a `FakeDevice` whose notification source reports
    /// when it is dropped (its reader thread ended) and whether it ever saw
    /// the device as gone.
    struct Watched {
        dev: FakeDevice,
        flags: Arc<Flags>,
        /// Stall every bulk read (a legacy unit whose image read fails).
        stall_bulk: bool,
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
            self.check(o)?;
            self.dev.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> dspi_transport::Result<Vec<u8>> {
            self.check(o)?;
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

    impl Watched {
        fn check(&self, opcode: u8) -> dspi_transport::Result<()> {
            let bulk = opcode == op::REQ_GET_ALL_PARAMS || opcode == op::REQ_GET_ALL_PARAMS_CHUNK;
            if self.stall_bulk && bulk {
                return Err(TransportError::Stalled { opcode });
            }
            Ok(())
        }
    }

    fn watched() -> (Watched, Arc<Flags>, FakeRemote) {
        watched_on(FakeDevice::demo(), false)
    }

    fn watched_on(dev: FakeDevice, stall_bulk: bool) -> (Watched, Arc<Flags>, FakeRemote) {
        let remote = dev.remote();
        let flags = Arc::new(Flags::default());
        (
            Watched {
                dev,
                flags: Arc::clone(&flags),
                stall_bulk,
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
        core.lock()
            .conn()
            .unwrap()
            .echo
            .bulk_write_started(Instant::now());
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
        close_instead: AtomicBool,
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
            let input = self.input.lock().unwrap().clone().unwrap();
            if self.close_instead.load(Ordering::SeqCst) {
                input.close_port();
                return Ok(());
            }
            let messages = self.decoder.lock().unwrap().feed(sysex);
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
            close_instead: AtomicBool::new(false),
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
        // The same handle (one per port) opens again: a fresh connection,
        // which the callback's `receive` now feeds.
        core.open_midi(handle.clone()).unwrap();
        assert!(core.lock().state.bulk_valid);
        core.disconnect();
        assert!(!core.is_connected());
        core.open_midi(handle).unwrap();
        assert!(core.is_connected());
        core.disconnect();
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1_open_midi_fails_cleanly_when_the_port_is_closed() {
        let core = DspiCore::new();
        let (bridge, handle) = midi_bridge();
        // The port goes away mid-connect: the platform calls close() from
        // the callback thread while the probe waits for a reply.
        bridge_closer(&bridge);
        assert!(core.open_midi(handle).is_err());
        assert!(!core.is_connected());
    }

    /// Makes the bridge close the port instead of answering.
    fn bridge_closer(bridge: &MidiBridge) {
        bridge.close_instead.store(true, Ordering::SeqCst);
    }

    /// The demo device answering with `image` as its bulk image.
    fn demo_with_image(image: Vec<u8>) -> FakeDevice {
        let options = dspi_transport::FakeOptions {
            extensions: true,
            ..Default::default()
        };
        let mut capture = dspi_transport::Capture::demo();
        capture.bulk = image;
        FakeDevice::new(capture, options)
    }

    fn assert_read_only(core: &DspiCore, compat: u8) {
        let mut inner = core.lock();
        assert!(!inner.state.bulk_valid);
        assert_eq!(inner.state.compat, compat);
        assert_eq!(inner.state.platform_id, 1, "from GET_PLATFORM");
        assert!(
            inner.state.fw_major >= 1,
            "firmware version from GET_PLATFORM"
        );
        let conn = inner.conn().unwrap();
        assert_eq!(conn.wire, None);
        assert!(!conn.writable());
        assert!(!conn.fresh_image);
        assert!(conn.session().is_err(), "no session on an unmappable image");
        // Plain reads still work (status, bootloader entry).
        assert!(conn.get_upto(op::REQ_GET_PLATFORM, 0, 7).is_ok());
        drop(inner);
        assert!(matches!(
            core.lock().writable_conn(),
            Err(DspiError::ReadOnly)
        ));
    }

    #[test]
    fn wp_c1_a_pre_v16_image_connects_read_only() {
        let mut image = dspi_transport::Capture::demo().bulk;
        image[0] = 15;
        let core = DspiCore::new();
        core.open_transport(Box::new(demo_with_image(image)), "OLD".into(), 0x2E8B)
            .unwrap();
        assert_read_only(&core, COMPAT_FIRMWARE_TOO_OLD);
        assert_eq!(core.lock().state.format_version, 15);
        assert_eq!(core.lock().selected.as_deref(), Some("OLD"));
    }

    #[test]
    fn wp_c1_an_implausible_newer_image_connects_read_only() {
        let mut image = dspi_transport::Capture::demo().bulk;
        image.resize(6200, 0);
        image[0] = 34;
        image[5] = 20; // band slots changed: the V32 prefix does not line up
        image[6..8].copy_from_slice(&6200u16.to_le_bytes());
        let core = DspiCore::new();
        core.open_transport(Box::new(demo_with_image(image)), "NEW".into(), 0)
            .unwrap();
        assert_read_only(&core, COMPAT_FIRMWARE_TOO_NEW);
        assert_eq!(core.lock().state.format_version, 34);
    }

    #[test]
    fn wp_c1_a_legacy_unit_whose_image_read_fails_connects_read_only() {
        let core = DspiCore::new();
        let (dev, _, _) = watched_on(FakeDevice::demo(), true);
        core.open_transport(Box::new(dev), "LEGACY".into(), LEGACY_USB_VID)
            .unwrap();
        assert_read_only(&core, COMPAT_FIRMWARE_TOO_OLD);
        assert!(core.lock().conn().unwrap().notifications.is_some());
        // The same failure on a current-VID unit is an error.
        let (dev, _, _) = watched_on(FakeDevice::demo(), true);
        assert!(matches!(
            core.open_transport(Box::new(dev), "CURRENT".into(), 0x2E8B),
            Err(DspiError::Transfer { .. })
        ));
        assert!(!core.is_connected());
    }

    #[test]
    fn wp_c1_a_good_connect_marks_the_image_fresh() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        assert!(core.lock().conn().unwrap().fresh_image);
    }

    struct Counter(AtomicUsize);

    impl NotifyListener for Counter {
        fn on_notifications(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn wp_c1_the_listener_is_woken_once_after_the_install() {
        let core = DspiCore::new();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        core.listeners.notify.set(Some(counter.clone()));
        core.open_demo().unwrap();
        assert!(counter.0.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn wp_c1_scan_quiet_finds_nothing_without_usb() {
        assert!(DspiCore::new().scan_quiet().is_empty());
    }

    #[test]
    fn wp_c1_attach_probed_fills_the_firmware_identity() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let mut want = state::default_state();
        state::apply_platform(&dspi_transport::Capture::demo().platform, &mut want).unwrap();
        let inner = core.lock();
        let s = &inner.state;
        assert_eq!(
            (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
            (want.fw_major, want.fw_minor, want.fw_patch, want.fw_beta)
        );
        assert_eq!(
            (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
            (1, 1, 6, 5)
        );
        assert_eq!(s.platform_id, 1);
        assert_eq!(s.platform_id, want.platform_id);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_f1 {
    use super::*;

    struct Sender(std::result::Result<(), DspiError>, Mutex<Vec<Vec<u8>>>);

    impl MidiSender for Sender {
        fn send_sysex(&self, sysex: Vec<u8>) -> Result<()> {
            self.1.lock().unwrap().push(sysex);
            self.0.clone()
        }
    }

    #[test]
    fn wp_f1_a_foreign_sender_is_a_pipe() {
        let ok = Arc::new(Sender(Ok(()), Mutex::new(Vec::new())));
        let pipe = SenderPipe(ok.clone());
        assert_eq!(pipe.send(&[0xF0, 0xF7]), Ok(()));
        assert_eq!(*ok.1.lock().unwrap(), vec![vec![0xF0, 0xF7]]);
    }

    #[test]
    fn wp_f1_a_foreign_sender_error_maps_to_the_pipe_error() {
        let closed = SenderPipe(Arc::new(Sender(
            Err(DspiError::Disconnected),
            Mutex::new(Vec::new()),
        )));
        assert_eq!(closed.send(&[]), Err(PipeError::Closed));
        let failed = SenderPipe(Arc::new(Sender(
            Err(DspiError::Io { msg: "busy".into() }),
            Mutex::new(Vec::new()),
        )));
        assert_eq!(failed.send(&[]), Err(PipeError::Failed("busy".into())));
    }
}
