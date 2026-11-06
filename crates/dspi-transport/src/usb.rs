//! USB backend, over `nusb`.
//!
//! `nusb` talks WinUSB, IOKit and Linux usbfs directly, with no libusb to
//! install. The firmware ships MS OS 2.0 descriptors, so Windows binds WinUSB to
//! the vendor interface automatically and no Zadig step is needed.
//!
//! All transfers go through a **claimed interface** rather than the device
//! handle: `nusb`'s device-level control transfers are unsupported on Windows,
//! and WinUSB additionally requires that the low byte of `wIndex` match the
//! claimed interface number. The protocol already requires `wIndex == 2`, so
//! these agree; this module simply never lets them disagree.
//!
//! **Android** has no bus enumeration (`nusb::list_devices` does not exist
//! there, and apps cannot read `/dev/bus/usb`). The app opens the device
//! through `UsbManager` and hands its usbfs descriptor to
//! [`UsbTransport::from_fd`]; [`list_devices`] is empty and the `open_*`
//! constructors return [`TransportError::NotFound`].
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::time::Duration;

use dspi_proto::generated::usb as usb_gen;
use dspi_proto::{REQ_TYPE_IN, REQ_TYPE_OUT, USB_PID, USB_VID, VENDOR_INTERFACE};
use nusb::MaybeFuture;
use nusb::transfer::{Bulk, ControlIn, ControlOut, ControlType, In, Recipient, TransferError};

use crate::{
    DEFAULT_TIMEOUT, DeviceDescriptor, NotificationSource, Result, Transport, TransportError,
};

/// `VENDOR_EP_IN` (config.h): the notification endpoint. It is a bulk
/// endpoint despite the spec's name; an interrupt endpoint polled alongside
/// rapid EP0 traffic crashes the RP2xxx device controller.
const NOTIFY_EP: u8 = usb_gen::VENDOR_EP_IN as u8;
const NOTIFY_EP_SIZE: usize = usb_gen::VENDOR_EP_SIZE as usize;

/// The notification endpoint, opened on a clone of the claimed interface so
/// the control path and this reader can live on different threads.
pub struct UsbNotifications {
    endpoint: nusb::Endpoint<Bulk, In>,
}

impl NotificationSource for UsbNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let buf = self.endpoint.allocate(NOTIFY_EP_SIZE);
        let done = self.endpoint.transfer_blocking(buf, timeout);
        match done.status {
            Ok(()) => {
                let mut v = done.buffer.into_vec();
                v.truncate(done.actual_len);
                Ok(v)
            }
            Err(TransferError::Cancelled) => Ok(Vec::new()),
            Err(TransferError::Disconnected) => Err(TransportError::Disconnected),
            Err(TransferError::Stall) => Err(TransportError::Stalled { opcode: NOTIFY_EP }),
            Err(e) => Err(TransportError::Usb(e.to_string())),
        }
    }
}

/// The vendor interface number, as a `u8` for `claim_interface`.
const IFACE: u8 = VENDOR_INTERFACE as u8;

/// Every connected DSPi: on Android always empty: apps cannot enumerate the bus (use
/// [`UsbTransport::from_fd`]).
#[cfg(target_os = "android")]
pub fn list_devices() -> Result<Vec<DeviceDescriptor>> {
    Ok(Vec::new())
}

/// Every connected DSPi, in a stable order, identified by serial.
///
/// A device with no readable serial is skipped rather than guessed at: identity
/// is what per-device settings and reconnection are keyed on, so an ambiguous
/// device is worse than an absent one.
#[cfg(not(target_os = "android"))]
pub fn list_devices() -> Result<Vec<DeviceDescriptor>> {
    let devices = nusb::list_devices()
        .wait()
        .map_err(|e| TransportError::Usb(e.to_string()))?;

    let mut found: Vec<DeviceDescriptor> = devices
        .filter(|d| is_dspi(d.vendor_id(), d.product_id()))
        .filter_map(|d| {
            Some(DeviceDescriptor {
                serial: d.serial_number()?.to_string(),
                bus_id: d.bus_id().to_string(),
                address: d.device_address(),
            })
        })
        .collect();

    found.sort_by(|a, b| a.serial.cmp(&b.serial));
    Ok(found)
}

pub struct UsbTransport {
    interface: nusb::Interface,
    descriptor: DeviceDescriptor,
    timeout: Duration,
}

impl UsbTransport {
    /// Open the only connected DSPi, or fail if there is not exactly one.
    ///
    /// With several attached, the caller must choose: silently picking one would
    /// mean a script could reconfigure the wrong unit.
    pub fn open_only() -> Result<Self> {
        let mut devices = list_devices()?;
        match devices.len() {
            0 => Err(TransportError::NotFound),
            1 => Self::open_serial(&devices.remove(0).serial),
            _ => Err(TransportError::Usb(format!(
                "{} DSPi devices connected; select one with --device <serial>",
                devices.len()
            ))),
        }
    }

    /// Open the DSPi with this serial number.
    ///
    /// On Android this is always [`TransportError::NotFound`]: apps cannot
    /// enumerate the bus (use [`UsbTransport::from_fd`]).
    #[cfg(target_os = "android")]
    pub fn open_serial(serial: &str) -> Result<Self> {
        let _ = serial;
        Err(TransportError::NotFound)
    }

    /// Open the DSPi with this serial number.
    #[cfg(not(target_os = "android"))]
    pub fn open_serial(serial: &str) -> Result<Self> {
        let devices = nusb::list_devices()
            .wait()
            .map_err(|e| TransportError::Usb(e.to_string()))?;

        let info = devices
            .filter(|d| is_dspi(d.vendor_id(), d.product_id()))
            .find(|d| d.serial_number() == Some(serial))
            .ok_or_else(|| TransportError::SerialNotFound(serial.to_string()))?;

        let descriptor = DeviceDescriptor {
            serial: serial.to_string(),
            bus_id: info.bus_id().to_string(),
            address: info.device_address(),
        };

        let device = info.open().wait().map_err(map_open_error)?;
        let interface = device
            .claim_interface(IFACE)
            .wait()
            .map_err(map_open_error)?;

        Ok(Self {
            interface,
            descriptor,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Transport for UsbTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        debug_assert_eq!(REQ_TYPE_IN, 0xC1, "IN is vendor|interface|device-to-host");

        let data = self
            .interface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: opcode,
                    value,
                    // Never a parameter. The protocol reserves wIndex for the
                    // interface number, and WinUSB enforces the same thing.
                    index: VENDOR_INTERFACE,
                    length: len,
                },
                self.timeout,
            )
            .wait()
            .map_err(|e| map_transfer_error(e, opcode))?;

        // A short read means the device answered a different question than the
        // one we asked. Treating it as success would decode garbage.
        if data.len() < len as usize {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: len as usize,
                got: data.len(),
            });
        }

        Ok(data)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.interface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: opcode,
                    value,
                    index: VENDOR_INTERFACE,
                    length: max_len,
                },
                self.timeout,
            )
            .wait()
            .map_err(|e| map_transfer_error(e, opcode))
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        debug_assert_eq!(REQ_TYPE_OUT, 0x41, "OUT is vendor|interface|host-to-device");

        self.interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: opcode,
                    value,
                    index: VENDOR_INTERFACE,
                    data,
                },
                self.timeout,
            )
            .wait()
            .map_err(|e| map_transfer_error(e, opcode))?;

        Ok(())
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let endpoint = self
            .interface
            .clone()
            .endpoint::<Bulk, In>(NOTIFY_EP)
            .ok()?;
        Some(Box::new(UsbNotifications { endpoint }))
    }

    fn max_transfer(&self) -> usize {
        // WinUSB caps a single control transfer at 4 KB. The other backends
        // manage larger, but the bulk packet exceeds 4 KB everywhere, so the
        // chunked path is used unconditionally rather than only on Windows.
        // One code path gets exercised on every platform, which is the point.
        4096
    }
}

fn map_open_error(e: nusb::Error) -> TransportError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("permission") || lower.contains("access") || lower.contains("denied") {
        TransportError::PermissionDenied
    } else {
        // Keep the underlying text: "could not open" covers a dozen distinct
        // causes and discarding the detail makes them indistinguishable.
        TransportError::Usb(msg)
    }
}

fn map_transfer_error(e: TransferError, opcode: u8) -> TransportError {
    match e {
        // A stall is the firmware refusing the request: an unknown opcode, a bad
        // index, an oversized payload, or a handler that returned failure. It is
        // also what a transfer hitting a flash blackout looks like, which is why
        // the retry helper treats it as possibly transient.
        TransferError::Stall => TransportError::Stalled { opcode },
        TransferError::Disconnected => TransportError::Disconnected,
        TransferError::Cancelled => TransportError::TimedOut { opcode, retries: 0 },
        other => TransportError::Usb(other.to_string()),
    }
}

/// The vendor ID older firmware enumerated with (the Raspberry Pi VID). The
/// old core accepted it, so an un-updated device still shows up (and probe
/// tells it to update); enumeration and `from_fd` share the rule.
const LEGACY_USB_VID: u16 = 0x2E8A;

/// Whether a device descriptor's IDs name a DSPi (current or legacy VID).
fn is_dspi(vid: u16, pid: u16) -> bool {
    pid == USB_PID && (vid == USB_VID || vid == LEGACY_USB_VID)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl UsbTransport {
    /// Open a device from a file descriptor the platform already opened:
    /// Android's `UsbManager.openDevice()` (`UsbDeviceConnection.fileDescriptor`),
    /// where nothing can enumerate the bus. Takes ownership of `fd`: the
    /// caller passes a `dup(2)` of the platform's descriptor, so Kotlin can
    /// still close its own `UsbDeviceConnection`. Claims the vendor
    /// interface only (never the audio interfaces 0 and 1, and no kernel
    /// driver is detached).
    ///
    /// `serial` is trusted from the caller (Android:
    /// `UsbDevice.getSerialNumber()`); the descriptor's `bus_id` is empty and
    /// its `address` 0, since a bare usbfs descriptor names neither.
    ///
    /// Errors: [`TransportError::NotFound`] if the descriptor's device is not
    /// a DSPi (VID `2E8B` or the legacy `2E8A`, PID `FEAA`);
    /// [`TransportError::PermissionDenied`] or [`TransportError::Usb`] if
    /// `fd` is not a usable usbfs descriptor or the interface cannot be
    /// claimed. `fd` is closed on every error path.
    pub fn from_fd(fd: std::os::fd::OwnedFd, serial: &str) -> Result<Self> {
        let device = nusb::Device::from_fd(fd).wait().map_err(map_open_error)?;

        let desc = device.device_descriptor();
        if !is_dspi(desc.vendor_id(), desc.product_id()) {
            return Err(TransportError::NotFound);
        }

        let interface = device
            .claim_interface(IFACE)
            .wait()
            .map_err(map_open_error)?;

        Ok(Self {
            interface,
            descriptor: DeviceDescriptor {
                serial: serial.to_string(),
                bus_id: String::new(),
                address: 0,
            },
            timeout: DEFAULT_TIMEOUT,
        })
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t1 {
    use super::*;

    #[test]
    fn wp_t1_is_dspi_accepts_current_and_legacy_vid_only() {
        assert!(is_dspi(USB_VID, USB_PID));
        assert!(is_dspi(LEGACY_USB_VID, USB_PID));
        assert!(!is_dspi(USB_VID, USB_PID ^ 1));
        assert!(!is_dspi(0x1234, USB_PID));
        assert!(!is_dspi(0, 0));
    }

    /// A scratch directory next to the test binary (under the cargo target
    /// directory), never tmpfs.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn scratch(name: &str) -> std::path::PathBuf {
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap().join("wp-t1-scratch");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// A USB device descriptor (18 bytes) with these IDs, as usbfs serves it
    /// at offset 0 of the device node, followed by a minimal configuration.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn descriptors(vid: u16, pid: u16) -> Vec<u8> {
        let [vl, vh] = vid.to_le_bytes();
        let [pl, ph] = pid.to_le_bytes();
        let mut d = vec![
            18, 1, 0x00, 0x02, 0, 0, 0, 64, vl, vh, pl, ph, 0x00, 0x01, 1, 2, 3, 1,
        ];
        // Configuration 1, no interfaces.
        d.extend_from_slice(&[9, 2, 9, 0, 0, 1, 0, 0x80, 50]);
        d
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn open_with(bytes: &[u8], name: &str) -> std::fs::File {
        let path = scratch(name);
        std::fs::write(&path, bytes).unwrap();
        std::fs::File::open(&path).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn wp_t1_from_fd_on_a_regular_file_is_a_usb_error_not_a_panic() {
        let file = open_with(b"not a usb device", "garbage.bin");
        let res = UsbTransport::from_fd(file.into(), "SERIAL");
        assert!(matches!(res, Err(TransportError::Usb(_))));
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn wp_t1_from_fd_rejects_empty_descriptor_without_panicking() {
        let file = open_with(&[], "empty.bin");
        let res = UsbTransport::from_fd(file.into(), "SERIAL");
        assert!(matches!(res, Err(TransportError::Usb(_))));
    }

    /// A file that reads like a DSPi but is not a usbfs node: nusb parses the
    /// descriptors, then the device-node operations fail inside nusb (before
    /// the VID check, which only `wp_t1_is_dspi_*` covers without a device).
    /// That must be a `Usb` error, and adopting a duplicate leaves the caller's descriptor open
    /// (the contract `DspiCore::open_fd` relies on: Kotlin closes its own).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn wp_t1_from_fd_on_non_usbfs_fd_errors_and_keeps_callers_fd() {
        use std::io::{Read, Seek, SeekFrom};
        use std::os::fd::OwnedFd;

        let bytes = descriptors(USB_VID, USB_PID);
        let mut file = open_with(&bytes, "dspi-like.bin");
        let dup: OwnedFd = file.try_clone().unwrap().into();
        let res = UsbTransport::from_fd(dup, "SERIAL");
        assert!(matches!(res, Err(TransportError::Usb(_))));

        // The caller's own descriptor is untouched.
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut back = Vec::new();
        file.read_to_end(&mut back).unwrap();
        assert_eq!(back, bytes);
    }
}
