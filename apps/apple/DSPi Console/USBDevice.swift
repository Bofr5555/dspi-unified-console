//
//  USBDevice.swift
//  DSPi Console
//
//  -- WP SW-M1 -- What remains of the app's IOKit USB driver: the device
//  model every transport lists.  The Mac reaches its local DSPi through the
//  Rust core's USB now (Core/CoreUsb.swift).
//

import Foundation

// MARK: - Device Model

/// Where a device other than a local USB one lives: the hub sharing it and
/// the handle it is addressed by there.  Handle 255 (the reserved value)
/// means "not yet known", as for a device listed from a hub's TXT record
/// before connecting to it.
struct RemoteHubRef: Hashable {
    let hubID: String
    let hubName: String
    var handle: UInt8
}

struct DSPiDevice: Identifiable {
    let serial: String      // the USB serial number
    let locationID: UInt32  // the bus location (IOKit's locationID on macOS)
    /// Set for a device reached through a DSPi Link hub; nil for local USB.
    var hub: RemoteHubRef? = nil
    /// The hub-stored friendly name, when reached through a hub.
    var remoteName: String? = nil
    var id: String { serial }
    var displayName: String {
        let short = String(serial.suffix(8))
        // A local device has no stored name unless a transport gives it one
        // (the iOS demo device does); USB devices never do.
        guard let hub = hub else { return remoteName ?? "DSPi (\(short))" }
        return "\(remoteName ?? "DSPi (\(short))") via \(hub.hubName)"
    }
    var isRemote: Bool { hub != nil }
}

extension DSPiDevice: Hashable {
    static func == (lhs: DSPiDevice, rhs: DSPiDevice) -> Bool {
        lhs.serial == rhs.serial
    }
    func hash(into hasher: inout Hasher) {
        hasher.combine(serial)
    }
}
