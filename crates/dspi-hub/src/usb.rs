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

// -- WP ATHUB --
/// The host bus of each USB device seen at enumeration (`nusb`'s
/// `DeviceInfo::bus_id`), by serial: `system.measure.open`'s gate G1 asks
/// whether USB-direct units share one host bus (PLAN-AT R7a).
static BUSES: std::sync::Mutex<std::collections::BTreeMap<String, String>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

// -- WP ATHUB --
#[cfg_attr(
    not(all(feature = "usb", not(target_os = "android"))),
    allow(dead_code)
)]
fn keep_bus(serial: &str, bus: &str) {
    let mut b = BUSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // At most a few hundred devices: the map stays small.
    if b.len() < 1024 || b.contains_key(serial) {
        b.insert(serial.to_owned(), bus.to_owned());
    }
}

// -- WP ATHUB --
/// The host bus the device `serial` was last enumerated on, where the
/// platform gives one; `None` for a device not on USB or a platform without.
pub(crate) fn bus_of(serial: &str) -> Option<String> {
    BUSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(serial)
        .filter(|b| !b.is_empty())
        .cloned()
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
                // -- WP ATHUB -- the host bus, kept for G1's USB check
                if let Some(serial) = d.serial_number().filter(|s| !s.is_empty()) {
                    keep_bus(serial, d.bus_id());
                }
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
        let raw = dspi_transport::UsbTransport::open_serial(serial)?;
        // -- WP PGWIRE -- the generation probe first, on the raw wire
        dspi_transport::paged::open(Box::new(raw))
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
