//
//  TestHostGuard.swift
//  DSPi Console
//
//  WP SW2.  The unit tests run inside the app (the test host), so the app's
//  own start-up would open a DSPi that happens to be attached: USB on macOS,
//  the USB MIDI control port on both targets.  Under XCTest it opens no real
//  device unless the run asks for one, and the app sees an empty bus instead.
//
//  Opt in per run with one of the hardware tests' variables:
//  `TEST_RUNNER_DSPI_USB_HW=1 xcodebuild test ...` (USB, HardwareTest),
//  `DSPI_MIDI_HW=1` or `DSPI_EXT_HW=1` (MIDI hardware tests).
//

import Foundation

enum TestHostGuard {
    /// Set in the test host's environment by xcodebuild for unit tests.
    static let underXCTest = ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil

    /// The variables that let a test run reach a real DSPi.
    static let hardwareOptIns = ["DSPI_USB_HW", "DSPI_MIDI_HW", "DSPI_EXT_HW"]

    /// False under XCTest unless one of `hardwareOptIns` is 1.
    static let mayOpenDevices: Bool = {
        guard underXCTest else { return true }
        let env = ProcessInfo.processInfo.environment
        return hardwareOptIns.contains { env[$0] == "1" }
    }()

    /// The MIDI port for the app's transport: the real one, or an empty bus.
    static func midiPort() -> MIDIControlPort {
        guard mayOpenDevices else { return NoDeviceMIDIPort() }
        return MIDIRelayPort.fromDefaults() ?? CoreMIDIControlPort()
    }
}

/// A MIDI port with no DSPi on it, ever: no CoreMIDI client, no relay.
final class NoDeviceMIDIPort: MIDIControlPort {
    let endpoints: [MIDIControlEndpoint] = []
    var onEndpointsChanged: (() -> Void)?
    var onReceive: (([UInt32]) -> Void)?
    let unavailableReason: String? = nil

    func open(_ endpoint: MIDIControlEndpoint) -> Bool { false }
    func close() {}
    func send(_ words: [UInt32]) -> Bool { false }
}
