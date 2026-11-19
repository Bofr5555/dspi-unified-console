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
use crate::records::{BulkImageExtras, DeviceInfo, DspState};
use crate::{Connection, DspiCore, DspiError, Result, rta, state};

/// The legacy USB vendor id of firmware before May 2026 (`USB_VID` is the
/// current one); the PID is the same.
pub(crate) const LEGACY_USB_VID: u16 = 0x2E8A;

/// Whether a USB device's ids name a DSPi (current or legacy VID).
#[cfg(any(test, all(feature = "usb", not(target_os = "android"))))]
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

// -- WP PGFIX --
/// The product strings of the alink sender build (`firmware/DSPi/
/// usb_descriptors.c` with `DSPI_ROLE_SENDER`; its first name too).
pub(crate) const SENDER_PRODUCTS: [&str; 2] = ["DSPi Audio Link Sender", "DSPi Link Sender"];

// -- WP PGFIX --
/// Whether a device is the alink sender (`firmware/DSPi-Sender`), from its
/// `REQ_GET_PLATFORM` reply and, where the descriptor is at hand, its USB
/// product string, as PGZERO tells boards apart: the product string decides
/// when there is one; without it (MIDI, behind a hub) a platform that names
/// no output channels is the sender's (`sender_vendor.c`: byte 3 is 0, "no
/// output channels: a sender"; every DSPi has outputs). A reply that does not
/// decode is not the sender's.
pub(crate) fn is_alink_sender_platform(platform: &[u8], product: Option<&str>) -> bool {
    let Ok(info) = PlatformInfo::decode(platform) else {
        return false;
    };
    match product.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => SENDER_PRODUCTS.contains(&p),
        None => info.num_output_channels == 0,
    }
}

// -- WP PGSENDER --
/// The pages the dual-stack alink sender lists (FWPG4, extension channel spec
/// 1.3: `SYS.PROTOCOL_INFO`, `SYS.PLATFORM`, its vendor subset on the paged
/// stack and the tunnel): SYS (identity, discovery, bootloader), STORE (its
/// output-config save) and our private page 0xE0.
pub(crate) const SENDER_PAGES: [u8; 3] = [
    dspi_proto::paged::cmds::SYS_PLATFORM.page(),
    dspi_proto::paged::cmds::STORE_ALL_PARAMS.page(),
    dspi_proto::paged::EXT_PAGE,
];

// -- WP PGSENDER --
/// Whether a paged device is the dual-stack alink sender, from what
/// `paged::open` read at connect (PLAN-PG section 1.9). Every condition must
/// hold; anything else is a DSP or an unrecognised device, never a sender
/// (fail closed):
/// 1. it identifies as the sender as on the legacy stack: `SYS.PLATFORM` has
///    the 1.1.6 `GET_PLATFORM` layout, so [`is_alink_sender_platform`] (the
///    product string where there is one, else no outputs);
/// 2. it is our firmware: page 0xE0 with the tunnel `0xE010` (the sender's
///    alink block lives on the extension channel);
/// 3. it has no bulk image: `STORE.ALL_PARAMS` GET is not in its bitmaps;
/// 4. it lists no page but SYS, STORE and 0xE0 ([`SENDER_PAGES`]): a device
///    with any DSP page is not the sender.
pub(crate) fn is_paged_alink_sender(
    disc: &dspi_transport::paged::Discovery,
    product: Option<&str>,
) -> bool {
    use dspi_proto::paged::{PagedDir, cmds};
    is_alink_sender_platform(&disc.platform, product)
        && disc.has_extensions()
        && !disc.pages.contains(cmds::STORE_ALL_PARAMS, PagedDir::Get)
        && disc.pages.pages().all(|p| SENDER_PAGES.contains(&p.page))
}

// -- WP PGFIX --
/// The connection to an alink sender whose bulk read failed (`err`), or
/// `None` when the device is not one: the sender's vendor subset has no bulk
/// image and no DSP model (PGMAT F1). On the legacy generation the platform
/// and product decide ([`is_alink_sender_platform`]); on the paged one (the
/// dual-stack sender, FWPG4) the connection's discovery must say so too
/// ([`is_paged_alink_sender`], WP PGSENDER), and a paged connection without
/// a discovery is not a sender (fail closed). What it has is read here:
/// `GET_PLATFORM` (the mirror's identity, `bulk_valid` false, no channels)
/// and the extension channel ([`ExtState::probe`], through the tunnel on the
/// paged stack; refused or absent, it is empty). `Err(Disconnected)` when
/// the device went meanwhile.
pub(crate) fn sender_mirror(
    t: &mut dyn Transport,
    err: &TransportError,
) -> Option<Result<(DspState, dspi_session::ext_state::ExtState)>> {
    if matches!(err, TransportError::Disconnected) {
        return None;
    }
    // -- WP PGSENDER -- the paged stack: the discovery decides first, so
    // nothing more is sent to a paged device that is not the sender
    if matches!(t.generation(), Some(dspi_transport::Generation::Paged(_))) {
        let product = t.descriptor().product.clone();
        let is_sender = t
            .paged_discovery()
            .is_some_and(|d| is_paged_alink_sender(&d, product.as_deref()));
        if !is_sender {
            return None;
        }
    }
    let reply = t
        .control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_REQUEST_LEN)
        .ok()?;
    if !is_alink_sender_platform(&reply, t.descriptor().product.as_deref()) {
        return None;
    }
    let mut mirror = state::default_state();
    state::apply_platform(&reply, &mut mirror).ok()?;
    mirror.num_channels = 0;
    mirror.num_input_channels = 0;
    mirror.num_output_channels = 0;
    mirror.bulk_valid = false;
    let ext = match dspi_session::ext_state::ExtState::probe(t) {
        Ok(ext) => ext,
        Err(TransportError::Disconnected) => return Some(Err(DspiError::Disconnected)),
        Err(e) => {
            log::warn!("the sender's extension probe failed, continuing without it: {e}");
            dspi_session::ext_state::ExtState::default()
        }
    };
    log::info!("{err}: the alink sender, connected with its own commands only");
    Some(Ok((mirror, ext)))
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

/// `wLength` of the `GET_PLATFORM` read: -- WP PGFWUP -- 8 bytes, the longest
/// reply (a DSPi-Zero's board id in byte 7; PLAN-PG section 1.9).
const PLATFORM_REQUEST_LEN: u16 = {
    assert!(PlatformInfo::FULL_LEN <= u16::MAX as usize);
    PlatformInfo::FULL_LEN as u16
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
        drop(self.set_meter_path(None)); // -- WP LK2 --
        if let Some(conn) = &old {
            self.events.disconnected(&conn.serial, "closed"); // -- WP P0M --
        }
        if let Some(worker) = worker {
            worker.stop();
        }
        if let Some(conn) = old {
            shut(conn);
        }
        self.kick_link();
    }

    /// The last step of the connect sequence: install `conn` and `mirror`,
    /// then, with the lock released, stop whatever another thread installed
    /// meanwhile (the last open wins) and wake the [`crate::NotifyListener`]
    /// once, for packets that arrived during the connect.
    pub(crate) fn install(&self, conn: Connection, mirror: DspState, serial: String, vid: u16) {
        let reader = conn.notifications.is_some();
        // -- WP P0M --
        let vreg = conn
            .ext
            .core_vreg
            .as_ref()
            .map(|v| (v.uptime_s, v.hang_count));
        // -- WP CS1 -- what `reset_info_at_connect` answers.
        let reset = conn
            .ext
            .reset_info
            .as_ref()
            .map(crate::ext_diag::reset_info);
        let opened = crate::events::Opened::new(&conn.serial, conn.caps.kind, &mirror, vreg, reset);
        let (displaced, worker) = {
            let mut inner = self.lock();
            inner.state = mirror;
            inner.selected = (vid != 0).then_some(serial);
            inner.cs = crate::cs::CsState::default();
            let worker = rta::take_worker(&mut inner);
            inner.confirm.connected(conn.generation); // -- WP P1BFIX -- F5
            inner.flash_target = None; // -- WP PGFLASH -- another device's story
            inner.demo_generation = None; // -- WP PGQT -- the fake of another opening
            (inner.conn.replace(conn), worker)
        };
        drop(self.set_meter_path(None)); // -- WP LK2 --
        self.limits_preload(); // -- WP P1CFIX -- N1: the profile's files, unlocked
        // -- WP P0M --
        if let Some(conn) = &displaced {
            self.events.disconnected(&conn.serial, "replaced");
        }
        self.events.connected(opened);
        if let Some(worker) = worker {
            worker.stop();
        }
        if let Some(old) = displaced {
            shut(old);
        }
        self.kick_link();
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
    /// errors from the connect sequence. Linux and Android only, with the
    /// `usb` feature; [`DspiError::Unsupported`] elsewhere.
    pub fn open_fd(&self, fd: i32, serial: String, vid: u16) -> Result<()> {
        #[cfg(all(feature = "usb", any(target_os = "linux", target_os = "android")))]
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
            // -- WP PGWIRE -- the generation probe first, on the raw wire
            let transport = dspi_transport::paged::open(Box::new(transport))?;
            self.attach_probed(transport, serial, vid)
        }
        #[cfg(not(all(feature = "usb", any(target_os = "linux", target_os = "android"))))]
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
    /// cannot enumerate (use `open_fd`), and in a build without the `usb`
    /// feature (iOS). Never fails: an enumeration error gives an empty list.
    pub fn scan_devices(&self) -> Vec<DeviceInfo> {
        // -- WP HB0 -- A broker owns USB: its list stands in for the bus.
        if let Some(broker) = self.broker() {
            let (devices, inaccessible) = (broker.devices(), broker.inaccessible());
            let mut inner = self.lock();
            inner.devices = devices.clone();
            inner.inaccessible = inaccessible;
            return devices;
        }
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
    /// [`DspiCore::open_fd`]) and in a build without the `usb` feature (iOS).
    /// With a [`crate::DeviceBroker`] set, the broker opens it instead, on any
    /// platform.
    pub fn select_device(&self, serial: String) -> Result<()> {
        // -- WP HB0 -- A broker owns USB: never claim the interface here.
        if let Some(broker) = self.broker() {
            return self.select_brokered(&*broker, serial);
        }
        #[cfg(any(not(feature = "usb"), target_os = "android"))]
        {
            let _ = serial;
            Err(DspiError::Unsupported)
        }
        #[cfg(all(feature = "usb", not(target_os = "android")))]
        {
            self.lock().selected = Some(serial.clone());
            // Close first: the device may be the one we hold, and a second
            // claim of its interface 2 would fail.
            self.close_connection();
            let vid = usb::vendor_of(&serial).ok_or(DspiError::Disconnected)?;
            let transport = dspi_transport::UsbTransport::open_serial(&serial)?;
            // -- WP PGWIRE -- the generation probe first, on the raw wire
            let transport = dspi_transport::paged::open(Box::new(transport))?;
            self.attach_probed(transport, serial, vid)
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
    // -- WP QD6 --
    /// The `wire_compat::feat` bits the connected firmware's wire format carries:
    /// every known bit on V32 and newer, none with no device or below V16.
    pub fn wire_features(&self) -> u32 {
        dspi_proto::wire_compat::features(self.lock().state.format_version)
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
    /// `MidiManager`, iOS CoreMIDI): a fresh connection over `input`'s pipe,
    /// then the connect sequence, while the app's MIDI callback feeds
    /// `input`. Replaces any open device. The same handle may be opened
    /// again (a reconnect over the same port). Errors as for the connect
    /// sequence (a silent device is a timeout).
    pub fn open_midi(&self, input: Arc<MidiInputHandle>) -> Result<()> {
        let transport = input.connect();
        let serial = input.serial.clone();
        // -- WP PGWIRE -- the generation probe first, on the raw wire
        let transport = dspi_transport::paged::open(Box::new(transport))?;
        self.attach_probed(transport, serial, 0)
    }
}

// -- WP QD6 --
impl DspiCore {
    /// Open the demo device as firmware of wire format `wire`, for clients'
    /// read-only tests ([`dspi_transport::Capture::demo_as_wire`]). Replaces any
    /// open device. [`DspiError::Unsupported`] for a format without a fixture.
    pub fn open_demo_wire(&self, wire: u8) -> Result<()> {
        let demo = FakeDevice::demo_as_wire(wire).ok_or(DspiError::Unsupported)?;
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial, 0)
    }

    // -- WP PGQT --
    /// Open the demo device as firmware of protocol generation `code`, for
    /// clients' smoke runs (the Qt driver's sections per generation): 0 G1s
    /// (stock 1.1.6), 1 G1b with PAGEGUARD, 2 G1b without it, 3 G2s (stock
    /// 1.1.7), 4 G2b (our dual-stack 1.1.7), 5 the DSPi-Zero, 6 the alink
    /// sender, 7 G1b with PAGEGUARD on a board at the stock 1.15 V core (a
    /// Pico on our firmware, which the voltage rule alone clears), 8 the
    /// dual-stack alink sender (paged 1.1.7 and 1.1.6; WP QTSENDER). The demo's
    /// own core voltage is 1.20 V. Opened through the generation probe as a USB device is
    /// (`dspi_transport::paged::open`). A request the generation must not get
    /// is counted ([`DspiCore::demo_generation_violations`]), never a panic.
    /// Replaces any open device. [`DspiError::Unsupported`] for another code.
    pub fn open_demo_generation(&self, code: u8) -> Result<()> {
        use dspi_transport::fake::FakeGeneration as G;
        let dev = match code {
            0 => FakeDevice::demo_generation(G::Stock116),
            1 | 7 => FakeDevice::demo_generation(G::Bofr116 { page_guard: true }),
            2 => FakeDevice::demo_generation(G::Bofr116 { page_guard: false }),
            3 => FakeDevice::demo_generation(G::STOCK117),
            4 => FakeDevice::demo_generation(G::Bofr117),
            5 => FakeDevice::zero(),
            6 => FakeDevice::sender(false),
            8 => FakeDevice::sender(true), // -- WP QTSENDER --
            _ => return Err(DspiError::Unsupported),
        };
        let remote = dev.remote();
        remote.allow_violations();
        if code == 7 {
            remote.vreg_set_build_default(0x0C); // 1.15 V
        }
        let serial = dev.descriptor().serial.clone();
        let transport = dspi_transport::paged::open(Box::new(dev))?;
        self.attach_probed(transport, serial, 0)?;
        self.lock().demo_generation = Some(remote);
        Ok(())
    }

    /// How many requests the fake of [`DspiCore::open_demo_generation`] got
    /// that its generation must not get (PLAN-PG section 1.1).
    /// [`DspiError::Unsupported`] when no such fake was opened on this core.
    pub fn demo_generation_violations(&self) -> Result<u32> {
        self.lock()
            .demo_generation
            .as_ref()
            .map(dspi_transport::FakeRemote::violations)
            .ok_or(DspiError::Unsupported)
    }

    /// How many writes this core has refused because the connection is read-only.
    /// A client's read-only mode keeps this from moving.
    pub fn read_only_refusals(&self) -> u32 {
        self.lock().read_only_refusals
    }
}

// -- WP QP11 -- (exported: WP QPI2)
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Whether the connect probe found feature `name` on the open device
    /// (`dspi_session::probe`'s names: `spdif_multi_input`, `adat_input`,
    /// `i2s_slave_clock`, ...), so a client hides what the firmware lacks
    /// without comparing versions. `i2s_clock_pin_mode`, which has no probe,
    /// answers whether the parameter image carried the clock-pin mode. False
    /// with no device, before the probe, and for a name the probe does not
    /// know. -- WP CG-SWS-a -- The names with no probe (`first_order_pass`,
    /// `subharmonic_extended`, `stereo_upmixer`, ...) are answered from the
    /// wire format, firmware version and platform the connect read;
    /// [`known_feature_names`] lists every name.
    pub fn feature_present(&self, name: &str) -> bool {
        self.ensure_probed();
        let mut inner = self.lock();
        if name == "i2s_clock_pin_mode" {
            return inner.conn.is_some() && inner.state.i2s_clock_pin_mode_present;
        }
        // -- WP CG-SWS-a -- the names with no probe of their own
        if let Some(present) = derived_feature(&inner.state, name) {
            return inner.conn.is_some() && present;
        }
        let Ok(conn) = inner.conn() else {
            return false;
        };
        conn.session().is_ok_and(|s| {
            s.capabilities()
                .features
                .iter()
                .any(|f| f.name == name && f.present)
        })
    }
}

// -- WP CG-SWS-a --
/// The probe's feature names (`dspi_session::probe`), in its order.
const PROBED_FEATURES: [&str; 23] = [
    "input_source",
    "loudness_output_mask",
    "crossfeed_output_mask",
    "leveller_masks",
    "psychoacoustic_bass",
    "upmixer",
    "adat_output",
    "adat_input",
    "i2s_slave_clock",
    "i2s_input_channels",
    "spdif_multi_input",
    "lg_sound_sync",
    "dac_hardware_mute",
    "uart_control",
    "i2c_control",
    "test_signals",
    "subharmonic_synth",
    "tube_preamp",
    "output_limiter",
    "spectrum_analyser",
    "input_source_names",
    "device_name",
    "output_latency",
];

// -- WP CG-SWS-a --
/// The feature names with no probe of their own, which
/// [`DspiCore::feature_present`] answers from what the connect read: the
/// parameter image's wire format, the firmware version and the platform.
/// Each replaces a client's version test (the Mac's `firmwareSupports*`,
/// `DSPViewModel.swift:2116-2232`, 2288-2310):
///
/// - `crossover_filters`: V11 (every format the core reads);
/// - `first_order_allpass`: V13; `first_order_shelves`: V14;
/// - `first_order_pass`: V28, the first that surely has first-order low and
///   high pass (`wire_compat::feat::SPDIF_IN4`);
/// - `linkwitz_transform`: V22 (`feat::LINKWITZ`);
/// - `upmixer_presence`: V26 (`feat::UPMIX_PRESENCE`);
/// - `stereo_upmixer`: V26 on an RP2350, the Mac's whole upmixer gate (the
///   RP2040 answers the probe but stalls the writes);
/// - `subharmonic_extended`: V30, the subharmonic synthesizer's third band,
///   selectivity, depth, hold, ceiling and pair link (`feat::SUBHARM_EXT`;
///   Qt's gate on that bit), whose reads stall on V29;
/// - `notch_filter`, `allpass_filter`, `band_bypass`: firmware 1.1.4.
const DERIVED_FEATURES: [&str; 12] = [
    "crossover_filters",
    "first_order_allpass",
    "first_order_shelves",
    "first_order_pass",
    "linkwitz_transform",
    "upmixer_presence",
    "stereo_upmixer",
    "subharmonic_extended",
    "notch_filter",
    "allpass_filter",
    "band_bypass",
    "i2s_clock_pin_mode",
];

/// A [`DERIVED_FEATURES`] name's answer from `s`, `None` for another name.
/// `i2s_clock_pin_mode` is answered by its own branch in `feature_present`.
fn derived_feature(s: &DspState, name: &str) -> Option<bool> {
    use dspi_proto::wire_compat::{feat, features};
    let v = s.format_version;
    let bits = features(v);
    let fw_114 = (s.fw_major, s.fw_minor, s.fw_patch) >= (1, 1, 4);
    Some(match name {
        "crossover_filters" => v >= 11,
        "first_order_allpass" => v >= 13,
        "first_order_shelves" => v >= 14,
        "first_order_pass" => bits & feat::SPDIF_IN4 != 0,
        "linkwitz_transform" => bits & feat::LINKWITZ != 0,
        "upmixer_presence" => bits & feat::UPMIX_PRESENCE != 0,
        "stereo_upmixer" => bits & feat::UPMIX_PRESENCE != 0 && s.platform_id == 1,
        "subharmonic_extended" => bits & feat::SUBHARM_EXT != 0,
        "notch_filter" | "allpass_filter" | "band_bypass" => fw_114,
        _ => return None,
    })
}

// -- WP CG-SWS-a --
/// Every name [`DspiCore::feature_present`] knows: the connect probe's, then
/// [`DERIVED_FEATURES`]. A client gates each section by one of these, never
/// by a wire format or firmware version.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn known_feature_names() -> Vec<String> {
    PROBED_FEATURES
        .iter()
        .chain(DERIVED_FEATURES.iter())
        .map(|n| (*n).to_owned())
        .collect()
}

// -- WP CG-SWS-a --
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// [`crate::bulk_image_extras`] of the mirror's parameter image: the
    /// bytes `DspState` has no field for. All false with no image.
    pub fn image_extras(&self) -> BulkImageExtras {
        let raw = self.lock().state.bulk_raw.clone();
        crate::bulk_image_extras(raw)
    }
}

impl DspiCore {
    /// The DSPis on USB, by enumeration only: nothing is opened or claimed,
    /// so a device another program holds is listed like any other and
    /// `inaccessible_devices` is not updated. For C1b's hotplug diffs, which
    /// run every half second; [`DspiCore::scan_devices`] is the picker's
    /// scan. Only reads the broker from `Inner`. Empty on Android and without the
    /// `usb` feature, unless a broker (WP HB0) owns the devices.
    pub(crate) fn scan_quiet(&self) -> Vec<DeviceInfo> {
        match self.broker() {
            Some(broker) => broker.devices(),
            None => usb::scan_quiet(),
        }
    }
}

// -- WP SW-M1 --
/// The DSPis on USB by enumeration only, for [`crate::usb_pipe`]: as the
/// hotplug poll's scan, without a broker. Empty on Android and without `usb`.
pub(crate) fn usb_scan_quiet() -> Vec<DeviceInfo> {
    usb::scan_quiet()
}

/// USB enumeration through nusb; empty on Android and without the `usb`
/// feature (WP CG3).
mod usb {
    use crate::records::DeviceInfo;
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Every DSPi on the bus, and how many could not be opened. `ours`
    /// (the device the core holds) is listed without opening it.
    #[cfg(any(not(feature = "usb"), target_os = "android"))]
    pub(super) fn scan(ours: Option<&str>) -> (Vec<DeviceInfo>, u32) {
        let _ = ours;
        (Vec::new(), 0)
    }

    /// Every DSPi on the bus, and how many could not be opened. `ours`
    /// (the device the core holds) is listed without opening it. A device
    /// without a serial is skipped before any open: identity is what
    /// selection and reconnection are keyed on.
    #[cfg(all(feature = "usb", not(target_os = "android")))]
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
    #[cfg(all(feature = "usb", not(target_os = "android")))]
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

    /// Nothing to enumerate on Android, or without the `usb` feature.
    #[cfg(any(not(feature = "usb"), target_os = "android"))]
    pub(super) fn scan_quiet() -> Vec<DeviceInfo> {
        Vec::new()
    }

    /// Enumeration failed last time (see [`dspis`]).
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    static LIST_FAILING: AtomicBool = AtomicBool::new(false);

    /// The DSPis on the bus (empty, logged, if enumeration fails).
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    fn dspis() -> impl Iterator<Item = nusb::DeviceInfo> {
        use nusb::MaybeFuture;
        let devices = match nusb::list_devices().wait() {
            Ok(d) => Some(d),
            Err(e) => {
                // Polled every half second: a warning when it starts failing,
                // then quiet until it has worked again.
                if !LIST_FAILING.swap(true, Ordering::Relaxed) {
                    log::warn!("USB enumeration failed: {e}");
                } else {
                    log::debug!("USB enumeration failed: {e}");
                }
                None
            }
        };
        if devices.is_some() {
            LIST_FAILING.store(false, Ordering::Relaxed);
        }
        devices
            .into_iter()
            .flatten()
            .filter(|d| super::is_dspi(d.vendor_id(), d.product_id()))
    }

    /// The device's serial, if it has a non-empty one.
    #[cfg(all(feature = "usb", not(target_os = "android")))]
    fn serial_of(d: &nusb::DeviceInfo) -> Option<String> {
        d.serial_number()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// The vendor id the DSPi with this serial is on the bus under.
    #[cfg(all(feature = "usb", not(target_os = "android")))]
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
    #[cfg(all(feature = "usb", not(target_os = "android")))]
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
    #[cfg(all(feature = "usb", not(target_os = "android")))]
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
#[cfg(all(feature = "usb", windows))]
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
        if cfg!(any(target_os = "android", not(feature = "usb"))) {
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

    #[cfg(all(feature = "usb", any(target_os = "linux", target_os = "android")))]
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

    /// Without the `usb` feature the USB entry points stay in the API (one
    /// binding for every platform) and report that there is no USB.
    #[cfg(not(feature = "usb"))]
    #[test]
    fn wp_cg3_without_usb_the_api_stays_and_refuses() {
        let core = DspiCore::new();
        assert!(core.scan_devices().is_empty());
        assert!(core.scan_quiet().is_empty());
        assert_eq!(core.inaccessible_devices(), 0);
        assert!(matches!(
            core.select_device("X".into()),
            Err(DspiError::Unsupported)
        ));
        assert!(matches!(
            core.open_fd(0, "X".into(), 0x2E8B),
            Err(DspiError::Unsupported)
        ));
        assert!(!core.is_connected());
        let counts = crate::bootloader_counts();
        assert_eq!((counts.rp2040, counts.rp2350), (0, 0));
    }

    /// The other transports do not depend on the `usb` feature.
    #[test]
    fn wp_cg3_demo_device_opens_with_or_without_usb() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        assert!(core.is_connected());
        core.disconnect();
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
            (1, 1, 6, 0),
            "the demo runs final 1.1.6 (WP CG27)"
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

// -- WP QP11 --
#[cfg(test)]
mod tests_wp_qp11 {
    use super::*;

    #[test]
    fn wp_qp11_no_device_has_no_features() {
        let core = DspiCore::new();
        assert!(!core.feature_present("input_source"));
        assert!(!core.feature_present("i2s_clock_pin_mode"));
    }

    #[test]
    fn wp_qp11_features_follow_the_probe() {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        // The demo is current firmware: the hardware pages' features answer
        for name in [
            "input_source",
            "spdif_multi_input",
            "i2s_input_channels",
            "dac_hardware_mute",
            "output_latency",
        ] {
            assert!(core.feature_present(name), "{name}");
        }
        assert!(core.feature_present("i2s_clock_pin_mode"));
        assert!(!core.feature_present("no_such_feature"));
        assert!(!core.feature_present(""));
    }
}

// -- WP FLAKE1 --
#[cfg(test)]
impl DspiCore {
    /// [`DspiCore::open_demo`] with its notifications read on each drain
    /// (`Notifications::caller_driven` over the demo's own endpoint) instead
    /// of on a reader thread: what the demo has sent is then in the core's
    /// queue as soon as the write that sent it returns, however late a loaded
    /// host would have scheduled the thread. For tests whose subject is what
    /// a drain, or a settle, sees.
    pub(crate) fn open_demo_read_on_drain(&self) -> Result<()> {
        let demo = FakeDevice::demo();
        let serial = demo.descriptor().serial.clone();
        let source = demo.notifications();
        self.attach_probed(Box::new(demo), serial, 0)?;
        // What the thread read during the open is applied first; the thread
        // then stops (dropping its reader joins it) before anything is written.
        let _ = self.process_notifications();
        if let Some(conn) = self.lock().conn.as_mut() {
            conn.notifications = Some(dspi_session::Notifications::caller_driven(source));
        }
        Ok(())
    }
}

// -- WP CG-SWS-a --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "device/tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;

// -- WP PGFIX --
#[cfg(test)]
#[path = "device/tests_wp_pgfix.rs"]
mod tests_wp_pgfix;

// -- WP PGQT --
#[cfg(test)]
#[path = "device/tests_wp_pgqt.rs"]
mod tests_wp_pgqt;

// -- WP PGSENDER --
#[cfg(test)]
#[path = "device/tests_wp_pgsender.rs"]
mod tests_wp_pgsender;
