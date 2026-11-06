//! Devices: discovery, opening and closing, over every transport.
//!
//! Owned by WP C1 (docs/core-merge/PLAN.md); hotplug and reconnection are
//! WP C1b (`hotplug.rs`). Every `open_*` ends in
//! [`DspiCore::attach_probed`](crate::DspiCore), so the connect sequence is
//! the same on every transport.
//!
//! Thread rules (PLAN §2, C1): never join a thread (the notification
//! reader, the RTA worker) while holding the `Inner` lock; take the old
//! connection out under the lock, drop the guard, then drop/join it. Stop and
//! join the old reader before a new device is opened.
#![allow(unused_variables)]

use std::sync::Arc;

use dspi_transport::Transport;
use dspi_transport::midi::{MidiInput, MidiPipe};

#[allow(unused_imports)]
use crate::records::*;
use crate::{DspiCore, Result};

/// The receiving half of a MIDI connection ([`DspiCore::open_midi`]): the
/// app's MIDI receive callback hands every byte it gets to
/// [`MidiInputHandle::receive`], in any split.
pub struct MidiInputHandle(pub(crate) MidiInput);

impl MidiInputHandle {
    /// Feed received MIDI bytes (raw MIDI 1.0 stream, SysEx included).
    pub fn receive(&self, bytes: Vec<u8>) {
        self.0.receive(&bytes)
    }
}

impl DspiCore {
    /// Adopt a USB device the platform already opened: Android's
    /// `UsbDeviceConnection.getFileDescriptor()`. The core `dup(2)`s the fd
    /// and owns the duplicate, so the app keeps (and later closes) its own;
    /// it claims the vendor interface 2 only, never the audio interfaces.
    /// Linux and Android only; `Unsupported` elsewhere.
    pub fn open_fd(&self, fd: i32, serial: String, vid: u16) -> Result<()> {
        todo!("WP C1")
    }

    /// Close the device opened by [`DspiCore::open_fd`] (the same as
    /// [`DspiCore::disconnect`]); afterwards the app may close its
    /// `UsbDeviceConnection`. Not called `close` because UniFFI's Kotlin
    /// objects already have `close()` (free the object).
    pub fn close_fd(&self) {
        todo!("WP C1")
    }

    /// Enumerate DSPis on USB (nusb): VID `2E8B` or the legacy VID, PID
    /// `FEAA`, with a readable serial. Updates
    /// [`DspiCore::inaccessible_devices`]. Empty on Android, where apps
    /// cannot enumerate (use `open_fd`).
    pub fn scan_devices(&self) -> Vec<DeviceInfo> {
        todo!("WP C1")
    }

    /// Highest preamp (dB) the connected chip applies correctly: +18 on an
    /// RP2040 (its Q28 multiplier overflows above), +24 on an RP2350.
    pub fn preamp_max_db(&self) -> f32 {
        todo!("WP C1")
    }

    /// DSPis the last scan saw but could not open: no permission (Linux udev
    /// rule), no WinUSB binding, or held by another program.
    pub fn inaccessible_devices(&self) -> u32 {
        todo!("WP C1")
    }

    /// Open the USB DSPi with this serial and run the connect sequence.
    /// The serial is remembered for reconnection (WP C1b).
    pub fn select_device(&self, serial: String) -> Result<()> {
        todo!("WP C1")
    }

    /// Close the open device, whatever its transport. The selected serial is
    /// kept, so a replugged device reconnects (WP C1b).
    pub fn disconnect(&self) {
        todo!("WP C1")
    }

    /// Whether a device is open.
    pub fn is_connected(&self) -> bool {
        todo!("WP C1")
    }

    /// Open a device over a transport the embedder built (a Link device, a
    /// test double, a MIDI transport from `dspi_transport::midi::midir_port`)
    /// and run the connect sequence. Replaces any open device. `vid` is 0 for
    /// non-USB transports.
    pub fn open_transport(
        &self,
        transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        todo!("WP C1")
    }

    /// Open the built-in demo device (`dspi_transport::FakeDevice::demo`):
    /// a simulated RP2350 that answers reads and writes and notifies.
    pub fn open_demo(&self) -> Result<()> {
        todo!("WP C1 (needs WP T4)")
    }

    /// Open a DSPi through the USB MIDI control channel, over a pipe the app
    /// supplies (Android `MidiManager`, iOS CoreMIDI). The returned handle
    /// is what the app's MIDI receive callback feeds; dropping it does not
    /// close the device ([`DspiCore::disconnect`] does).
    pub fn open_midi(
        &self,
        pipe: Arc<dyn MidiPipe>,
        serial: String,
    ) -> Result<Arc<MidiInputHandle>> {
        todo!("WP C1 (needs WP T2)")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C1"]
    fn wp_c1_is_implemented() {
        todo!("WP C1")
    }
}
