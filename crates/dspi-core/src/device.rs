//! Devices: discovery, selection, hotplug, transports.
//!
//! Owned by WP C1 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use std::sync::Arc;

use dspi_transport::{MidiPipe, Transport};

use crate::callbacks::DeviceEvents;
#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

impl DspiCore {
    /// Adopt an already-open usbfs fd (`UsbDeviceConnection.getFileDescriptor()`).
    /// Claims interface 2 only (never the audio interfaces 0/1), starts the
    /// notify listener, resets state like `select_device`. The caller keeps
    /// the `UsbDeviceConnection` open until after `close_fd()`; rusb does not
    /// take ownership of the fd. Needs third_party/patches/0001-android-fd.patch
    /// (`DeviceManager::select_device_handle`, PLAN Appendix A).
    pub fn open_fd(&self, fd: i32, serial: String, vid: u16) -> Result<()> {
        todo!("WP C1")
    }

    /// Stop the listener/RTA threads and drop the device handle opened by
    /// `open_fd` (= `disconnect`), after which the Kotlin side may close the
    /// `UsbDeviceConnection`. Named `close_fd`, not `close`, because UniFFI's
    /// Kotlin object already has `close()` (AutoCloseable = free the core).
    pub fn close_fd(&self) {
        todo!("WP C1")
    }

    /// libusb enumeration. Always empty on Android (discovery disabled).
    pub fn scan_devices(&self) -> Vec<DeviceInfo> {
        todo!("WP C1")
    }

    /// Highest preamp (dB) the chip applies correctly: +18 RP2040, +24 RP2350.
    pub fn preamp_max_db(&self) -> f32 {
        todo!("WP C1")
    }

    pub fn inaccessible_devices(&self) -> u32 {
        todo!("WP C1")
    }

    pub fn select_device(&self, serial: String) -> Result<()> {
        todo!("WP C1")
    }

    pub fn disconnect(&self) {
        todo!("WP C1")
    }

    pub fn is_connected(&self) -> bool {
        todo!("WP C1")
    }

    /// Register (Some) or clear (None) the arrival/departure listener.
    /// Clearing waits for a call in progress on another thread; replacing
    /// (Some -> Some) does not wait.
    pub fn set_hotplug_callback(&self, listener: Option<Arc<dyn DeviceEvents>>) {
        todo!("WP C1")
    }

    /// libusb poll (~500 ms on desktop). No-op on Android.
    pub fn poll_hotplug(&self) {
        todo!("WP C1")
    }

    /// Open a device over a transport the embedder built: a MIDI port pair,
    /// a Link device, a test double. Replaces any open device. `vid` is 0
    /// for non-USB transports.
    pub fn open_transport(
        &self,
        transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        let _ = (transport, serial, vid);
        todo!("WP C1")
    }

    /// Open the built-in demo device (`dspi_transport::FakeDevice::demo`).
    pub fn open_demo(&self) -> Result<()> {
        todo!("WP C1 (needs WP T4)")
    }

    /// Open a DSPi through the USB MIDI control channel, over a pipe the app
    /// supplies (Android `MidiManager`, iOS CoreMIDI). Returns the input
    /// handle the app's MIDI receive callback feeds.
    pub fn open_midi(
        &self,
        pipe: Box<dyn MidiPipe>,
        serial: String,
    ) -> Result<dspi_transport::midi::MidiInput> {
        let _ = (pipe, serial);
        todo!("WP C1 (needs WP T2)")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "WP C1"]
    fn wp_c1_is_implemented() {
        todo!("WP C1")
    }
}
