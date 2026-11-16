//! Listener interfaces the embedding app implements.
//!
//! They are called on core threads (the notification reader, the RTA
//! worker, the Link session's supervisor) or, for [`DeviceEvents`], on the thread that calls
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
/// with its serial, whatever the transport (USB, MIDI, Link; a Link session
/// that is reconnecting does not count yet, see `link_status`). This is the
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
    /// The session ended for good: revoked, a protocol error, or refused by
    /// the hub. A lost connection is not an end: the core reconnects with
    /// backoff and reports it through
    /// [`LinkListener::on_link_status_changed`]. `code`: the hub's WebSocket
    /// close code, -1 = none.
    fn on_link_closed(&self, code: i32, reason: String);
    /// `link_status().phase` changed, or a reconnect try failed: reconnecting,
    /// a PIN needed, no board, bound again (the mirror is then already
    /// fetched; re-read `get_state` and call `rta_start` again).
    fn on_link_status_changed(&self);
    /// The hub issued a token: store it now against `hub_id`. Comes as soon as
    /// the hub's answer arrives, even if `link_pair` has already timed out.
    /// -- WP SECCORE -- `token` is the credential to store (`v2` when pinned),
    /// and it also comes when a reconnect upgrades or re-pins it.
    fn on_link_token_issued(&self, hub_id: String, token: String);
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

// -- WP P5SES --
/// A measurement session's phase changed, it stopped, or it asks something:
/// read `measure_state()`. Called on the thread that pushed capture, ticked or
/// stopped, with the core lock released; must not block.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait MeasureListener: Send + Sync {
    /// The session's state changed.
    fn on_measure(&self);
}
