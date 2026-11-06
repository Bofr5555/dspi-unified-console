//! Low-level USB communication via libusb (rusb crate).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusb::{DeviceHandle, GlobalContext};

use crate::protocol::{REQ_TYPE_IN, REQ_TYPE_OUT};

/// The wIndex sent on the wire. Every request is addressed to the vendor
/// interface (recipient = interface). WinUSB only passes such a request to
/// the interface the handle owns, the vendor interface, so on Windows the
/// low byte always names it, as the Windows Console does. The firmware
/// ignores wIndex on vendor requests; elsewhere it is sent unchanged.
#[inline]
fn wire_index(index: u16) -> u16 {
    if cfg!(windows) {
        (index & 0xFF00) | crate::protocol::VENDOR_INTERFACE as u16
    } else {
        index
    }
}

/// Default USB transfer timeout.
const TIMEOUT: Duration = Duration::from_millis(1000);

/// Error type for USB operations.
#[derive(Debug, thiserror::Error)]
pub enum UsbError {
    #[error("USB transfer error: {0}")]
    Rusb(#[from] rusb::Error),
    #[error("Device not connected")]
    NotConnected,
    #[error("Short read: expected {expected}, got {actual}")]
    ShortRead { expected: usize, actual: usize },
    #[error("Invalid argument")]
    InvalidArgument,
}

pub type Result<T> = std::result::Result<T, UsbError>;

/// Wraps a rusb DeviceHandle for USB control transfers. The handle is shared
/// with the notification listener thread (libusb allows concurrent transfers).
pub struct UsbConnection {
    handle: Arc<DeviceHandle<GlobalContext>>,
    /// Held for a chunked bulk-parameter session, and by the spectrum
    /// analyser worker for each poll: any other vendor request aborts an open
    /// chunked session on the device.
    bus: Arc<Mutex<()>>,
}

impl UsbConnection {
    /// Create a new connection from an already-opened device handle.
    pub fn new(handle: DeviceHandle<GlobalContext>) -> Self {
        Self { handle: Arc::new(handle), bus: Arc::new(Mutex::new(())) }
    }

    /// The bus lock (see `bus`).
    pub fn bus(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.bus)
    }

    /// The shared device handle, for the notification listener.
    pub fn shared_handle(&self) -> Arc<DeviceHandle<GlobalContext>> {
        Arc::clone(&self.handle)
    }

    /// Send a vendor control transfer (Host→Device, fire-and-forget).
    /// bmRequestType = 0x41 (vendor, interface, host-to-device).
    pub fn send_control(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<()> {
        self.handle
            .write_control(REQ_TYPE_OUT, request, value, wire_index(index), data, TIMEOUT)?;
        Ok(())
    }

    /// Receive a vendor control transfer (Device→Host, synchronous).
    /// bmRequestType = 0xC1 (vendor, interface, device-to-host).
    /// Returns the received data.
    pub fn get_control(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; length as usize];
        let n = self
            .handle
            .read_control(REQ_TYPE_IN, request, value, wire_index(index), &mut buf, TIMEOUT)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Send a control transfer and verify the response has at least `min_len` bytes.
    pub fn get_control_exact(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        min_len: usize,
    ) -> Result<Vec<u8>> {
        let data = self.get_control(request, value, index, length)?;
        if data.len() < min_len {
            return Err(UsbError::ShortRead {
                expected: min_len,
                actual: data.len(),
            });
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::wire_index;

    #[test]
    fn wire_index_names_the_vendor_interface_on_windows() {
        if cfg!(windows) {
            assert_eq!(wire_index(0), 2);
            assert_eq!(wire_index(0x0100), 0x0102);
        } else {
            assert_eq!(wire_index(0), 0);
        }
        assert_eq!(wire_index(2), 2);
    }
}
