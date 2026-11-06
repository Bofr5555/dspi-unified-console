//! Listener interfaces the embedding app implements.
//!
//! They are called on core threads (the notification reader, the RTA
//! worker, the Link reader) or, for [`DeviceEvents`], on the thread that calls
//! `poll_hotplug`, with the core lock released. A listener may call the core's
//! read methods (`get_state`, `rta_get_snapshot`, `process_notifications`) but
//! must not block and must not open or close devices from inside the callback.
//!
//! The Android and iOS bindings (WP F1) export these as UniFFI foreign traits;
//! the C ABI wraps a C function pointer in each. A foreign callback that
//! throws is contained like a panicking listener: the call is lost, the core
//! goes on.

use crate::DspiError;

/// Notification packets are queued: call `process_notifications()` (the
/// apps batch it to at most one call per 30 ms).
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait NotifyListener: Send + Sync {
    /// Packets arrived. Called on the notification reader's thread.
    fn on_notifications(&self);
}

/// The analyser's capabilities or snapshot changed: call `rta_get_caps()` /
/// `rta_get_snapshot()`.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait RtaListener: Send + Sync {
    /// New capabilities or a new snapshot. Called on the RTA worker's thread.
    fn on_frame(&self);
}

/// Device arrival and departure, from `poll_hotplug`.
///
/// The device you were connected to going away always gives `on_departed`
/// with its serial, whatever the transport (USB, MIDI, Link). This is the
/// signal to rely on: `NOTIFY_DISCONNECTED` is only reported if
/// `process_notifications` ran before `poll_hotplug` closed the connection.
///
/// Call `scan_devices` before the first poll, or every unit present is
/// announced as arrived.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait DeviceEvents: Send + Sync {
    /// A DSPi with this serial appeared on the bus.
    fn on_arrived(&self, serial: String);
    /// The DSPi with this serial left the bus, or the open device was lost.
    fn on_departed(&self, serial: String);
}

/// DSPi Link session events.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait LinkListener: Send + Sync {
    /// The hub's device list changed (`link_devices()`).
    fn on_link_devices_changed(&self);
    /// The session ended. `code`: the hub's WebSocket close code, -1 = none.
    fn on_link_closed(&self, code: i32, reason: String);
}

/// How the app sends bytes to a DSPi's MIDI port (Android `MidiInputPort`,
/// iOS CoreMIDI), for [`crate::MidiInputHandle::with_sender`]. Called on the
/// thread that issues the request; must not block for long.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait MidiSender: Send + Sync {
    /// Send one complete SysEx message (`F0 .. F7`).
    ///
    /// Errors: [`DspiError::Disconnected`] when the port is closed or the
    /// device has gone; any other error fails the request in flight.
    fn send_sysex(&self, sysex: Vec<u8>) -> Result<(), DspiError>;
}
