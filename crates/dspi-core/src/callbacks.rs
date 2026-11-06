//! Listener interfaces the embedding app implements.
//!
//! They are called on core threads (the notification reader, the RTA
//! worker, the Link session's supervisor) or, for [`DeviceEvents`], on the thread that calls
//! `poll_hotplug`, with the core lock released. A listener may call the core's
//! read methods (`get_state`, `rta_get_snapshot`, `process_notifications`) but
//! must not block and must not open or close devices from inside the callback.
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
    fn on_link_token_issued(&self, hub_id: String, token: String);
}
