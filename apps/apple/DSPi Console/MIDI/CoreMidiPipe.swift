//
//  CoreMidiPipe.swift
//  DSPi Console
//
//  WP SW5.  The DSPi's USB MIDI control port through the Rust core: the
//  transport iPhone and iPad use (they cannot open the vendor interface,
//  owner decision D-iOS-1), and the Mac's fallback when the vendor interface
//  cannot be opened.
//
//  Only I/O is left in Swift.  The port (`MIDIControlPort`: CoreMIDI, or the
//  development relay `MIDIRelayPort`) moves UMP words; this file turns them
//  into the raw MIDI 1.0 SysEx bytes the core reads and back, and follows the
//  port's endpoints (hot-plug).  The core does the rest: the SysEx codec,
//  tags and request matching, timeouts and retries, notifications and the
//  echo filter (`dspi_transport::midi`, through `MidiInputHandle`).
//
//  * `MidiSender` (the core's send callback): one complete SysEx message to
//    the open endpoint, as UMP.  Called on the core's request thread.
//  * `MidiInputHandle`: one per open of an endpoint.  Every received message
//    goes to its `receive`; `closePort` when the endpoint goes away or this
//    pipe closes it, so a request in flight fails at once instead of waiting
//    out its timeout.
//  * `attach(_:to:)` runs the core's `openMidi` (the connect sequence) over
//    a handle; `CoreBackedTransport` calls it on its core queue.
//
//  `CoreBackedTransport(link: .midi(pipe))` is the `DeviceTransport` over
//  this pipe.  ci/check.sh keeps the core's MIDI handle in this file.
//

import Foundation
import os
import dspi_core

final class CoreMidiPipe {

    private static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "CoreMidiPipe")

    /// What a change of the port's endpoints means for the open device
    /// (MIDITransport.swift).  Delivered on the main thread; on `.departed`
    /// the handle is already closed.
    typealias Change = MIDIEndpointList.Change

    /// One open of an endpoint: the core's handle for it, opaque outside
    /// this file.
    struct Connection {
        fileprivate let handle: MidiInputHandle
    }

    /// How an `open` went.
    enum OpenResult {
        /// A fresh handle over the newly opened endpoint: run the core's
        /// connect sequence over it (`attach`).
        case opened(Connection)
        /// That device's endpoint is already open: keep it (and the probe
        /// that may be in progress on it).
        case alreadyOpen
        /// The port refused to open the endpoint.
        case failed(String)
        /// No endpoint for that device.
        case unknown
    }

    let port: MIDIControlPort
    /// Open the first DSPi that appears when nothing is selected.
    let autoSelect: Bool
    /// Hot-plug, on the main thread (`CoreBackedTransport` follows it).
    var onChange: ((Change) -> Void)?
    /// Called on the main thread after the device list changed.
    var onDevicesChanged: (([DSPiDevice]) -> Void)?

    /// Why MIDI is unusable (no CoreMIDI client), or nil.
    var unavailableReason: String? { port.unavailableReason }

    /// Guards everything below.
    private let lock = NSLock()
    private var list = MIDIEndpointList()
    private var openEndpoint: MIDIControlEndpoint?
    /// The open endpoint's handle; nil while closed.
    private var handle: MidiInputHandle?
    /// Bumped by every open and close: a sender from an earlier open
    /// refuses to send (its port is gone).
    private var openToken: UInt64 = 0
    /// Receive side only (one port thread at a time), reset on each open.
    private var assembler = SysExAssembler()

    var devices: [DSPiDevice] { lock.lock(); defer { lock.unlock() }; return list.devices }
    /// The device whose endpoint is open, if any.
    var openDevice: DSPiDevice? {
        lock.lock(); defer { lock.unlock() }
        return openEndpoint.map(Self.device(for:))
    }

    /// - Parameters:
    ///   - port: CoreMIDI, the relay, or a test fake.
    ///   - autoSelect: report a newly appeared DSPi as `.appeared`.
    init(port: MIDIControlPort, autoSelect: Bool = true) {
        self.port = port
        self.autoSelect = autoSelect
        port.onReceive = { [weak self] words in self?.received(words) }
        port.onEndpointsChanged = { [weak self] in self?.endpointsChanged() }
        refreshEndpoints()
    }

    deinit {
        close()
    }

    // MARK: - Devices

    static func device(for endpoint: MIDIControlEndpoint) -> DSPiDevice {
        MIDIEndpointList.device(for: endpoint)
    }

    /// Read the port's endpoints; returns what changed for the open one
    /// (and, with `autoSelect`, the first device that appeared).
    @discardableResult
    private func refreshEndpoints() -> (changes: [Change], devices: [DSPiDevice]) {
        let endpoints = port.endpoints
        lock.lock()
        var changes = list.update(endpoints, open: openEndpoint)
        let devices = list.devices
        lock.unlock()
        var offered = false
        changes = changes.filter { change in
            guard case .appeared = change else { return true }
            defer { offered = true }
            return autoSelect && !offered
        }
        return (changes, devices)
    }

    /// Main thread (the port's contract).
    private func endpointsChanged() {
        let (changes, devices) = refreshEndpoints()
        onDevicesChanged?(devices)
        for change in changes {
            if case .departed(let device) = change {
                Self.log.notice("DSPi MIDI port [\(device.serial, privacy: .public)] went away")
                close()
            }
            onChange?(change)
        }
    }

    // MARK: - Open and close

    /// Open `device`'s endpoint.  Re-selecting the device that is already
    /// open keeps it unless `force` (a reconnect, a replug).
    func open(_ device: DSPiDevice, force: Bool) -> OpenResult {
        lock.lock()
        let endpoint = list.bySerial[device.serial]
        let alreadyOpen = endpoint != nil && endpoint == openEndpoint && handle != nil
        lock.unlock()
        guard let endpoint = endpoint else { return .unknown }
        if alreadyOpen && !force { return .alreadyOpen }
        close()
        if let reason = port.unavailableReason { return .failed(reason) }
        guard port.open(endpoint) else {
            Self.log.error("Could not open DSPi MIDI port \(endpoint.name, privacy: .public)")
            return .failed("Could not open the DSPi's MIDI port \(endpoint.name).")
        }
        lock.lock()
        openToken &+= 1
        let sender = Sender(pipe: self, token: openToken)
        let fresh = MidiInputHandle.withSender(sender: sender, serial: endpoint.serial)
        openEndpoint = endpoint
        handle = fresh
        assembler.reset()
        lock.unlock()
        Self.log.notice("Opened DSPi MIDI port \(endpoint.name, privacy: .public) [\(endpoint.serial, privacy: .public)]")
        return .opened(Connection(handle: fresh))
    }

    /// Close the open endpoint: its handle's request in flight fails at once
    /// (`closePort`), and the port stops delivering.  Idempotent.
    func close() {
        lock.lock()
        let closing = handle
        let wasOpen = openEndpoint != nil
        handle = nil
        openEndpoint = nil
        openToken &+= 1
        lock.unlock()
        closing?.closePort()
        if wasOpen { port.close() }
    }

    /// The core's connect sequence over `connection` (blocking: the probe
    /// and the first reads).  Call it on the core's queue.
    func attach(_ connection: Connection, to core: DspiCore) throws {
        try core.openMidi(input: connection.handle)
    }

    // MARK: - I/O

    /// Port thread: UMP words to complete SysEx messages, to the open handle.
    private func received(_ words: [UInt32]) {
        lock.lock()
        let messages = assembler.feed(words)
        let target = handle
        lock.unlock()
        guard let target = target else { return }
        for message in messages { target.receive(bytes: Data(message)) }
    }

    /// The core's send, for the open with `token`.
    fileprivate func send(_ sysex: Data, token: UInt64) throws {
        lock.lock()
        let current = token == openToken && openEndpoint != nil
        lock.unlock()
        guard current else { throw DspiError.Disconnected }
        let words = MIDIUMPSysEx.words(forSysEx: [UInt8](sysex))
        guard port.send(words) else {
            // Closed meanwhile, or CoreMIDI refused the send.
            lock.lock(); let stillOpen = token == openToken && openEndpoint != nil; lock.unlock()
            if !stillOpen { throw DspiError.Disconnected }
            throw DspiError.Transfer(msg: "the MIDI port refused the send")
        }
    }

    /// The core's `MidiSender` for one open.  Holds the pipe weakly: the
    /// core keeps the sender for as long as the connection lives.
    private final class Sender: MidiSender, @unchecked Sendable {
        private weak var pipe: CoreMidiPipe?
        private let token: UInt64

        init(pipe: CoreMidiPipe, token: UInt64) {
            self.pipe = pipe
            self.token = token
        }

        func sendSysex(sysex: Data) throws {
            guard let pipe = pipe else { throw DspiError.Disconnected }
            try pipe.send(sysex, token: token)
        }
    }
}
