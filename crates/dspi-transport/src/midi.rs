//! The USB MIDI control channel: vendor requests carried in SysEx
//! (Bofr5555/DSPi `usb_midi_control_spec.md`; `DSPI_USB_MIDI` firmware only).
//!
//! Three layers:
//! - [`MidiClient`]: sans-IO. Turns a request into SysEx bytes and incoming
//!   bytes into replies and notifications. No port, no thread, no clock:
//!   usable from wasm and from a foreign event loop.
//! - [`MidiPipe`]: what the app (Kotlin `MidiManager`, Swift CoreMIDI) or
//!   [`midir_port`] supplies: a way to send bytes. Incoming bytes are pushed
//!   with [`MidiInput::receive`].
//! - [`MidiTransport`]: the blocking [`Transport`] over a pipe: one request
//!   in flight, matched by tag, with the spec's retry rules.
//!
//! Owned by WP T2 (codec client and transport) and WP T3 (`midir_port`).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;
use std::time::Duration;

use dspi_proto::frame::{Frame, Message};

use crate::{DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps};

/// Why a [`MidiPipe`] could not send.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipeError {
    /// The port is closed or the device went away.
    #[error("the MIDI port is closed")]
    Closed,
    /// The platform refused the send; the text is the platform's.
    #[error("MIDI send failed: {0}")]
    Failed(String),
}

/// How the host sends bytes to the device's MIDI IN port. Called from the
/// thread that issues requests; must not block for long. Shared (`Arc`) so a
/// binding can hand in a foreign object (a Kotlin or Swift callback).
pub trait MidiPipe: Send + Sync {
    /// Send one complete SysEx message (`F0 .. F7`).
    fn send(&self, sysex: &[u8]) -> std::result::Result<(), PipeError>;
}

/// What a received message means to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiEvent {
    /// The reply to the request with this tag.
    Reply { tag: u8, frame: Frame },
    /// A notify packet (tag 0), as the USB endpoint would deliver it.
    Notification(Vec<u8>),
    /// A message that matched our header but could not be read; fails the
    /// request in flight (tag `0x7F` or the request's own).
    Error { tag: u8, message: String },
}

/// The sans-IO client: tags, encoding, decoding.
pub struct MidiClient {
    #[allow(dead_code)]
    mfr: Vec<u8>,
}

impl MidiClient {
    /// `mfr`: the manufacturer ID the firmware was built with
    /// ([`dspi_proto::frame::DEFAULT_MFR_ID`] unless a product changed it).
    pub fn new(mfr: &[u8]) -> Self {
        Self { mfr: mfr.to_vec() }
    }

    /// Encode a request with the next tag (1..=0x7E). Returns the tag and the
    /// SysEx bytes to send.
    pub fn request(&mut self, frame: &Frame) -> (u8, Vec<u8>) {
        let _ = frame;
        todo!("WP T2")
    }

    /// Feed received bytes (any split); returns the events they completed.
    pub fn receive(&mut self, bytes: &[u8]) -> Vec<MidiEvent> {
        let _ = bytes;
        todo!("WP T2")
    }

    #[allow(dead_code)]
    fn classify(m: Message) -> MidiEvent {
        let _ = m;
        todo!("WP T2")
    }
}

/// The receiving half of a [`MidiTransport`]: the app's MIDI callback calls
/// [`MidiInput::receive`] with whatever bytes arrived.
#[derive(Clone)]
pub struct MidiInput {
    #[allow(dead_code)]
    shared: Arc<()>,
}

impl MidiInput {
    pub fn receive(&self, bytes: &[u8]) {
        let _ = bytes;
        todo!("WP T2")
    }
}

/// A blocking transport over a [`MidiPipe`].
pub struct MidiTransport {
    descriptor: DeviceDescriptor,
}

impl MidiTransport {
    /// Default per-request timeout: flash writes blind the device for about
    /// 45 ms and SysEx pacing varies by host (spec 4.6, 5).
    pub const TIMEOUT: Duration = Duration::from_millis(1500);

    /// A transport sending through `pipe`; feed received bytes to the
    /// returned [`MidiInput`]. `serial` names the device for the picker.
    pub fn new(pipe: Arc<dyn MidiPipe>, mfr: &[u8], serial: &str) -> (Self, MidiInput) {
        let _ = (pipe, mfr, serial);
        todo!("WP T2")
    }
}

impl Transport for MidiTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let _ = (opcode, value, len);
        todo!("WP T2")
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        let _ = (opcode, value, max_len);
        todo!("WP T2")
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        let _ = (opcode, value, data);
        todo!("WP T2")
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// No transfer cap: SysEx has no length limit and the bulk image goes in
    /// one message (spec 3.4).
    fn max_transfer(&self) -> usize {
        usize::MAX
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        todo!("WP T2")
    }

    /// `chunked_bulk: false`: the firmware serves `0xA2`/`0xA3` over MIDI
    /// (spec 4.2), but single-shot `0xA0`/`0xA1` are the recommended bulk
    /// commands there (one message, no chunk session to abort), so the
    /// session reads and writes the image in one request.
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: crate::TransportKind::Midi,
            chunked_bulk: false,
            remote: false,
        }
    }
}

/// Desktop MIDI ports through `midir` (ALSA, WinMM, CoreMIDI). WP T3.
#[cfg(feature = "midir")]
pub mod midir_port {
    /// A DSPi's MIDI port pair, by name ("DSPi Control").
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PortInfo {
        pub name: String,
    }

    /// The DSPi control ports visible to this host.
    pub fn list_ports() -> Vec<PortInfo> {
        todo!("WP T3")
    }

    /// Open a port pair and return a ready transport.
    pub fn open(port: &PortInfo) -> crate::Result<super::MidiTransport> {
        let _ = port;
        todo!("WP T3")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP T2"]
    fn a_get_round_trips_through_the_client() {
        todo!("WP T2")
    }

    #[test]
    #[ignore = "WP T2"]
    fn notifications_between_replies_are_delivered() {
        todo!("WP T2")
    }
}
