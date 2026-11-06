//! Finding and opening devices: the [`DeviceBus`] seam, and USB behind it.
//!
//! The hub's supervisor scans a bus and attaches what it finds, so a test drives
//! unplug and replug with a fake bus, and Android's fd-only world (a later sideload
//! build) needs only a different bus or `HubHandle::attach`.

use dspi_transport::{Transport, TransportError};

/// A device a bus can see. Nothing has been opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusDevice {
    /// Its serial.
    pub serial: String,
    /// USB vendor id: current or legacy firmware, 0 if not USB.
    pub vid: u16,
    /// Where it sits on the bus (`dspi_core::DeviceInfo::location_id`).
    pub location_id: u32,
}

/// Where a hub's devices come from. Called on the hub's supervisor thread only.
pub trait DeviceBus: Send {
    /// Every device present, whether or not it can be opened. Must not open any,
    /// and must be cheap: the supervisor calls it twice a second.
    ///
    /// `Err` is "cannot tell" and is never an empty bus: the hub then detaches nothing
    /// and attaches nothing until a later scan answers.
    fn scan(&mut self) -> Result<Vec<BusDevice>, TransportError>;

    /// Open and claim one device. Another program holding it, a missing permission
    /// or a missing driver is an error here; the hub tries again later.
    fn open(&mut self, serial: &str) -> Result<Box<dyn Transport>, TransportError>;
}

/// The USB bus, through nusb.
#[cfg(all(feature = "usb", not(target_os = "android")))]
#[derive(Debug, Default)]
pub struct UsbBus {
    failing: bool,
}

#[cfg(all(feature = "usb", not(target_os = "android")))]
impl DeviceBus for UsbBus {
    fn scan(&mut self) -> Result<Vec<BusDevice>, TransportError> {
        use nusb::MaybeFuture;
        let devices = match nusb::list_devices().wait() {
            Ok(devices) => {
                self.failing = false;
                devices
            }
            Err(e) => {
                // Polled: say so once when it starts failing.
                if !std::mem::replace(&mut self.failing, true) {
                    log::warn!("USB enumeration failed: {e}");
                }
                return Err(TransportError::Usb(e.to_string()));
            }
        };
        let mut found: Vec<BusDevice> = devices
            .filter(|d| {
                d.product_id() == dspi_proto::USB_PID
                    && (d.vendor_id() == dspi_proto::USB_VID || d.vendor_id() == LEGACY_USB_VID)
            })
            .filter_map(|d| {
                Some(BusDevice {
                    serial: d.serial_number().filter(|s| !s.is_empty())?.to_owned(),
                    vid: d.vendor_id(),
                    location_id: location_id(&d),
                })
            })
            .collect();
        found.sort_by(|a, b| a.serial.cmp(&b.serial));
        Ok(found)
    }

    fn open(&mut self, serial: &str) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(dspi_transport::UsbTransport::open_serial(serial)?))
    }
}

/// The vendor id of firmware before May 2026 (`dspi_core`'s `LEGACY_USB_VID`).
#[cfg(all(feature = "usb", not(target_os = "android")))]
const LEGACY_USB_VID: u16 = 0x2E8A;

/// As `dspi_core`'s picker reports it: IOKit's on macOS, else `address | bus << 8`.
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
