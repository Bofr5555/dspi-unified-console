//! Listener interfaces the embedding app implements.
//!
//! They are called on core threads (the notification reader, the RTA
//! worker, the Link reader) or, for [`DeviceEvents`], on the thread that calls
//! `poll_hotplug`. They must only *schedule* work and never call back into
//! [`crate::DspiCore`] from inside the callback.
//!
//! The Android and iOS bindings (WP F1) export these as UniFFI callback
//! interfaces; the C ABI wraps a C function pointer in each.

/// Notification packets are queued: call `process_notifications()` (the
/// apps batch it to at most one call per 30 ms).
pub trait NotifyListener: Send + Sync {
    fn on_notifications(&self);
}

/// The analyser's capabilities or snapshot changed: call `rta_get_caps()` /
/// `rta_get_snapshot()`.
pub trait RtaListener: Send + Sync {
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
pub trait DeviceEvents: Send + Sync {
    fn on_arrived(&self, serial: String);
    fn on_departed(&self, serial: String);
}

/// DSPi Link session events.
pub trait LinkListener: Send + Sync {
    /// The hub's device list changed (`link_devices()`).
    fn on_link_devices_changed(&self);
    /// The session ended. `code`: the hub's WebSocket close code, -1 = none.
    fn on_link_closed(&self, code: i32, reason: String);
}
