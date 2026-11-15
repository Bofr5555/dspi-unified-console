//
//  MIDITransport.swift
//  DSPi Console
//
//  -- WP SW5 -- The MIDI transport's I/O side, with no protocol in it: which
//  DSPi control ports exist and what a change of them means for the open
//  one (hot-plug), and the picker's row for each.  The port itself
//  (MIDIControlPort.swift: CoreMIDI; MIDIRelayPort.swift: the development
//  relay) moves UMP words; CoreMidiPipe.swift hands them to the Rust core,
//  which owns the control protocol (codec, tags, retries, notifications).
//
//  The Swift protocol engine that used to live here is
//  LegacyMIDITransport.swift, for the opt-out only.
//

import Foundation

/// The DSPi control ports a `MIDIControlPort` lists, followed across
/// updates.  A value type; its owner serialises the calls.
struct MIDIEndpointList {

    /// What an update means for the open endpoint.
    enum Change: Equatable {
        /// The open device's endpoint went away.
        case departed(DSPiDevice)
        /// The open device is listed again with new endpoint objects
        /// (replugged faster than the removal was seen): reopen it.
        case replugged(DSPiDevice)
        /// A device that was not listed at the previous update appeared.
        case appeared(DSPiDevice)
    }

    private(set) var bySerial: [String: MIDIControlEndpoint] = [:]
    private(set) var devices: [DSPiDevice] = []
    private var knownSerials: Set<String> = []

    /// The picker's row for an endpoint: the USB serial when CoreMIDI knows
    /// it, so the Mac's vendor and MIDI rows for one DSPi are one row.
    static func device(for endpoint: MIDIControlEndpoint) -> DSPiDevice {
        let name = endpoint.serial.hasPrefix("RELAY-")
            ? "DSPi via MIDI relay \(endpoint.serial.dropFirst(6))"
            : "DSPi (\(endpoint.serial.suffix(8))) via USB MIDI"
        return DSPiDevice(serial: endpoint.serial, locationID: endpoint.locationID, hub: nil, remoteName: name)
    }

    /// Take the port's current endpoints.  `open` is the endpoint open now,
    /// if any.  Departure or replug of it comes first, then the devices that
    /// just appeared (only those, so a device the user let go of, or
    /// switched away from, is not offered again).
    mutating func update(_ endpoints: [MIDIControlEndpoint], open: MIDIControlEndpoint?) -> [Change] {
        bySerial = Dictionary(endpoints.map { ($0.serial, $0) }, uniquingKeysWith: { a, _ in a })
        devices = endpoints.map(Self.device(for:))
        var changes: [Change] = []
        if let open = open {
            if let now = bySerial[open.serial] {
                if now != open { changes.append(.replugged(Self.device(for: now))) }
            } else {
                changes.append(.departed(Self.device(for: open)))
            }
        }
        for endpoint in endpoints where !knownSerials.contains(endpoint.serial) {
            changes.append(.appeared(Self.device(for: endpoint)))
        }
        knownSerials = Set(endpoints.map { $0.serial })
        return changes
    }
}
