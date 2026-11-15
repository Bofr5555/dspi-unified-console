import XCTest
import Combine
import dspi_core
@testable import DSPi_Console

/// WP SW5: the DSPi's USB MIDI port through the Rust core
/// (`CoreBackedTransport(link: .midi(CoreMidiPipe))`), against the fake MIDI
/// port and device of MIDITestSupport.swift.  The device behind the port is
/// Swift's demo device (`FakeTransport`), so the core's whole connect
/// sequence runs over SysEx; its PARAM_CHANGED echoes carry PARAM_SRC_MIDI,
/// as the firmware's do for a change made over MIDI.
final class CoreMidiSW5Tests: XCTestCase {

    private static let paramSrcMidi: UInt8 = 10
    private static let paramSrcHostSet: UInt8 = 1

    /// The demo device behind a MIDI port, echoes re-sourced to MIDI.
    private static func device(backedBy fake: FakeTransport) -> FakeMIDIDevice {
        let device = FakeMIDIDevice { r in
            if r.type == MIDIControlCodec.typeGet {
                switch fake.getControlResult(request: r.request, value: r.value, index: r.index, length: r.length) {
                case .success(let data): return (0, [UInt8](data))
                case .failure(let s): return (s.rawValue < 8 ? s.rawValue : 0x02, [])
                }
            }
            fake.sendControlRequest(request: r.request, value: r.value, index: r.index, data: Data(r.payload))
            return (0, [])
        }
        device.forwarding = fake.addNotificationObserver { [weak device] note in
            var packet = [UInt8](note.packet)
            if packet.count > 8, packet[1] == 0x02, packet[8] == paramSrcHostSet { packet[8] = paramSrcMidi }
            device?.notify(packet)
        }
        return device
    }

    private struct Rig {
        let transport: CoreBackedTransport
        let pipe: CoreMidiPipe
        let port: FakeMIDIControlPort
        let fake: FakeTransport
    }

    private func rig(autoSelect: Bool = true, connect: Bool = true,
                     file: StaticString = #filePath, line: UInt = #line) -> Rig {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(5) { fake.isConnected }, "the demo device is up", file: file, line: line)
        let port = FakeMIDIControlPort(device: Self.device(backedBy: fake))
        let pipe = CoreMidiPipe(port: port, autoSelect: autoSelect)
        let t = CoreBackedTransport(link: .midi(pipe))
        if connect {
            XCTAssertTrue(spinUntil(15) { t.isConnected }, "the core connects over the MIDI pipe", file: file, line: line)
        }
        return Rig(transport: t, pipe: pipe, port: port, fake: fake)
    }

    private func masterVolume(_ fake: FakeTransport) -> Float? {
        guard case .success(let d) = fake.getControlResult(request: REQ_GET_MASTER_VOLUME, value: 0, index: 2, length: 4),
              d.count == 4 else { return nil }
        return d.withUnsafeBytes { $0.loadUnaligned(as: Float.self) }
    }

    /// `body` off the main thread, the main run loop turning until it is done.
    private func offMain<T>(_ timeout: TimeInterval = 10, _ body: @escaping () -> T) -> T? {
        var out: T?
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).async { out = body(); done.signal() }
        let deadline = Date().addingTimeInterval(timeout)
        while done.wait(timeout: .now()) == .timedOut && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        return out
    }

    // MARK: - Connect, read, write

    func testSW5TheCoreConnectsOverTheMIDIPipeAndReadsTheDevice() throws {
        let r = rig()
        XCTAssertEqual(r.transport.availableDevices.map(\.serial), [FakeMIDIControlPort.endpoint.serial])
        XCTAssertEqual(r.transport.selectedDevice?.serial, FakeMIDIControlPort.endpoint.serial)
        XCTAssertEqual(r.transport.selectedDevice?.displayName.hasSuffix("via USB MIDI"), true)
        XCTAssertNil(r.transport.errorMessage)
        let source = try XCTUnwrap(r.transport.coreStateSource, "with the core attached")
        let state = try source.readAll().get()
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        XCTAssertEqual(state.masterVolumeDb, try XCTUnwrap(masterVolume(r.fake)), accuracy: 0.01)
        let seen = Set(r.port.device.requests.map(\.frame.request))
        XCTAssertTrue(seen.isSuperset(of: [REQ_GET_PLATFORM, REQ_GET_ALL_PARAMS]), "the core's probe and image read went over SysEx")
        // As on USB, wIndex is the vendor interface.
        XCTAssertTrue(r.port.device.requests.allSatisfy { $0.frame.index == 2 && $0.frame.crcOK })
    }

    func testSW5ATypedWriteReachesTheDeviceAndItsEchoIsOwn() throws {
        let r = rig()
        let source = try XCTUnwrap(r.transport.coreStateSource)
        var notes: [LinkNotification] = []
        let token = r.transport.addNotificationObserver { notes.append($0) }
        defer { token.cancel() }
        source.write(lane: nil) { try $0.setMasterVolume(db: -12) }
        XCTAssertTrue(spinUntil(5) { self.masterVolume(r.fake) == -12 }, "the typed write reached the device over MIDI")
        XCTAssertTrue(spinUntil(5) { notes.contains { $0.packet.count > 8 && $0.packet[1] == 0x02 } }, "its echo came back")
        let echo = try XCTUnwrap(notes.first { $0.packet.count > 8 && $0.packet[1] == 0x02 })
        XCTAssertEqual(echo.packet[8], Self.paramSrcMidi)
        XCTAssertEqual(echo.origin, r.transport.session, "PARAM_SRC_MIDI is our own write: the core's echo filter")
        XCTAssertTrue(spinUntil(5) { source.readState().masterVolumeDb == -12 })
        XCTAssertTrue(spinUntil(5) { source.readJournal().canUndo }, "the journal has the write")

        // Another host's change (USB, source HOST_SET) is not ours.
        notes.removeAll()
        r.port.device.notify(FakeMIDIDevice.paramChanged(seq: 77, source: Self.paramSrcHostSet, value: -3))
        XCTAssertTrue(spinUntil(5) { notes.contains { $0.packet.count > 3 && $0.packet[3] == 77 } })
        XCTAssertEqual(notes.first { $0.packet[3] == 77 }?.origin, 0)
    }

    func testSW5RawRequestsGoThroughTheCore() throws {
        let r = rig()
        let before = r.port.device.requests.count
        let got = offMain { r.transport.getControlResult(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4) }
        guard case .success(let data)? = got else { return XCTFail("raw GET: \(String(describing: got))") }
        XCTAssertEqual(data.count, 4)
        r.transport.sendControlRequest(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0,
                                       data: withUnsafeBytes(of: Float(-9)) { Data($0) })
        XCTAssertTrue(spinUntil(5) { self.masterVolume(r.fake) == -9 })
        let fresh = r.port.device.requests.dropFirst(before)
        XCTAssertTrue(fresh.contains { $0.frame.request == REQ_GET_MASTER_VOLUME && $0.frame.index == 2 },
                      "the core picks the interface")
        XCTAssertTrue(fresh.contains { $0.frame.request == REQ_SET_MASTER_VOLUME && $0.frame.payload.count == 4 })
    }

    // MARK: - The app's default

    func testSW5TheAppsMIDIIsTheCoresByDefault() throws {
        let d = UserDefaults.standard
        guard ProcessInfo.processInfo.environment["DSPI_CORE_TRANSPORT"] == nil,
              d.object(forKey: "CoreTransport") == nil, !d.bool(forKey: "MIDIReadOnly") else {
            throw XCTSkip("this run sets CoreTransport or MIDIReadOnly")
        }
        XCTAssertTrue(AppState.usesCoreTransport)
        XCTAssertFalse(AppState.usesLegacyMIDI)
        XCTAssertNotNil(AppState.shared.midiCore.midi, "the app's MIDI transport is the core's")
        #if os(iOS)
        XCTAssertTrue(AppState.shared.midiMember === AppState.shared.midiCore)
        #endif

        // The opt-out, and the read-only mode only the Swift engine has.
        d.set(false, forKey: "CoreTransport")
        XCTAssertTrue(AppState.usesLegacyMIDI)
        d.removeObject(forKey: "CoreTransport")
        d.set(true, forKey: "MIDIReadOnly")
        XCTAssertTrue(AppState.usesLegacyMIDI)
        d.removeObject(forKey: "MIDIReadOnly")
        XCTAssertFalse(AppState.usesLegacyMIDI)
    }

    // MARK: - Selection and hot-plug

    func testSW5ReselectingKeepsThePortAndDisconnectClosesIt() throws {
        let r = rig()
        let opens = r.port.openCount
        r.transport.selectDevice(try XCTUnwrap(r.transport.selectedDevice))
        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        XCTAssertEqual(r.port.openCount, opens, "the open port and its probe are kept")
        XCTAssertTrue(r.transport.isConnected)

        r.transport.disconnect()
        XCTAssertTrue(spinUntil(5) { !r.transport.isConnected })
        XCTAssertNil(r.transport.selectedDevice)
        XCTAssertNil(r.port.openEndpoint, "the CoreMIDI source is disconnected")
        XCTAssertNil(r.pipe.openDevice)
        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        XCTAssertFalse(r.transport.isConnected, "a device let go of is not reopened by itself")

        r.transport.reconnect()   // nothing selected: nothing happens
        RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        XCTAssertFalse(r.transport.isConnected)
        r.transport.selectDevice(MIDIEndpointList.device(for: FakeMIDIControlPort.endpoint))
        XCTAssertTrue(spinUntil(15) { r.transport.isConnected }, "selected again, it opens again")
    }

    func testSW5UnplugFailsTheRequestInFlightAtOnceAndReplugReconnects() throws {
        let r = rig()
        r.port.device.droppedReplies = 1   // the request in flight never gets an answer
        let started = Date()
        var result: Result<Data, DSPi_Console.LinkStatus>?
        let done = expectation(description: "the GET returns")
        DispatchQueue.global().async {
            result = r.transport.getControlResult(request: REQ_GET_MASTER_VOLUME, value: 0, index: 2, length: 4)
            done.fulfill()
        }
        let sent = r.port.device.requests.count
        XCTAssertTrue(spinUntil(5) { r.port.device.requests.count > sent })
        r.port.setEndpoints([])
        wait(for: [done], timeout: 5)
        XCTAssertLessThan(Date().timeIntervalSince(started), 1.2,
                          "closePort fails the request at once, before the core's 1.5 s timeout")
        if case .success? = result { XCTFail("a GET to a departed device cannot succeed") }
        XCTAssertTrue(spinUntil(5) { !r.transport.isConnected })
        XCTAssertNil(r.transport.selectedDevice)
        XCTAssertEqual(r.transport.availableDevices, [])

        r.port.setEndpoints([FakeMIDIControlPort.endpoint])
        XCTAssertTrue(spinUntil(15) { r.transport.isConnected }, "the replugged DSPi is opened again")
        XCTAssertEqual(r.transport.selectedDevice?.serial, FakeMIDIControlPort.endpoint.serial)
        XCTAssertNotNil(try XCTUnwrap(r.transport.coreStateSource).readAll().sw5Value)
    }

    func testSW5NewEndpointObjectsForTheOpenDeviceReopenIt() throws {
        let r = rig()
        let opens = r.port.openCount
        let e = FakeMIDIControlPort.endpoint
        let moved = MIDIControlEndpoint(serial: e.serial, locationID: e.locationID, name: e.name,
                                        sourceID: e.sourceID + 10, destinationID: e.destinationID + 10)
        r.port.setEndpoints([moved])
        XCTAssertTrue(spinUntil(5) { r.port.openCount > opens }, "reopened on the new endpoints")
        XCTAssertEqual(r.port.openEndpoint, moved)
        XCTAssertTrue(spinUntil(15) { r.transport.isConnected })
    }

    func testSW5WithoutAutoSelectItOpensOnlyWhenAsked() throws {
        // The Mac's fallback member: never opens the port by itself, so a
        // DAW's MIDI is left alone while the vendor interface works.
        let r = rig(autoSelect: false, connect: false)
        RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        XCTAssertNil(r.transport.selectedDevice)
        XCTAssertEqual(r.port.openCount, 0)
        XCTAssertEqual(r.transport.availableDevices.count, 1)
        r.transport.selectDevice(r.transport.availableDevices[0])
        XCTAssertTrue(spinUntil(15) { r.transport.isConnected })
    }

    func testSW5ASilentPortIsNotConnectedAndSaysSo() throws {
        let port = FakeMIDIControlPort(device: FakeMIDIDevice { _ in nil })
        let t = CoreBackedTransport(link: .midi(CoreMidiPipe(port: port)))
        XCTAssertTrue(spinUntil(5) { t.selectedDevice != nil })
        XCTAssertTrue(spinUntil(30) { t.errorMessage != nil }, "the probe gives up")
        XCTAssertFalse(t.isConnected)
        XCTAssertTrue(t.errorMessage?.hasPrefix("The DSPi's MIDI port does not answer") == true, t.errorMessage ?? "")
    }

    func testSW5APortThatCannotOpenSaysSo() throws {
        let port = RefusingPort()
        let t = CoreBackedTransport(link: .midi(CoreMidiPipe(port: port)))
        XCTAssertTrue(spinUntil(5) { t.errorMessage != nil })
        XCTAssertEqual(t.errorMessage, "Could not open the DSPi's MIDI port \(FakeMIDIControlPort.endpoint.name).")
        XCTAssertFalse(t.isConnected)
    }

    func testSW5ResumeRereadsWhenTheDeviceStillAnswers() throws {
        let r = rig()
        var notes: [LinkNotification] = []
        let token = r.transport.addNotificationObserver { notes.append($0) }
        defer { token.cancel() }
        r.transport.resumeMidi()
        XCTAssertTrue(spinUntil(5) { notes.contains { $0.packet == CoreBackedTransport.resyncPacket } },
                      "a BULK_INVALIDATED makes the view model re-read")
        XCTAssertTrue(r.transport.isConnected)
    }

    /// A port that lists the DSPi but cannot open it.
    private final class RefusingPort: MIDIControlPort {
        let endpoints = [FakeMIDIControlPort.endpoint]
        var onEndpointsChanged: (() -> Void)?
        var onReceive: (([UInt32]) -> Void)?
        let unavailableReason: String? = nil
        func open(_ endpoint: MIDIControlEndpoint) -> Bool { false }
        func close() {}
        func send(_ words: [UInt32]) -> Bool { false }
    }

    // MARK: - midi_vectors.json through the core

    private struct Vector {
        let name: String
        let hostToDevice: Bool
        let manufacturer: [UInt8]
        let tag: UInt8
        let frame: [UInt8]
        let sysex: [UInt8]
    }

    private static func hex(_ s: String) -> [UInt8] {
        var out: [UInt8] = []
        var i = s.startIndex
        while i < s.endIndex {
            let j = s.index(i, offsetBy: 2)
            out.append(UInt8(s[i..<j], radix: 16)!)
            i = j
        }
        return out
    }

    private func vectors() throws -> [String: Vector] {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "midi_vectors", withExtension: "json"))
        let text = try String(contentsOf: url, encoding: .utf8)
        var out: [String: Vector] = [:]
        for line in text.split(separator: "\n") where !line.isEmpty {
            let o = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any])
            let v = Vector(name: o["name"] as! String, hostToDevice: (o["dir"] as! String) == "h2d",
                           manufacturer: Self.hex(o["mfr"] as! String), tag: UInt8(o["tag"] as! Int),
                           frame: Self.hex(o["frame"] as! String), sysex: Self.hex(o["sysex"] as! String))
            out[v.name] = v
        }
        return out
    }

    /// `reply` (a device-to-host vector's SysEx) with its tag replaced by
    /// `tag`: the core picks its own tags.  The tag byte follows
    /// F0, the manufacturer ID, "DSP" and the version.
    private static func retag(_ reply: Vector, _ tag: UInt8) -> [UInt8] {
        var sysex = reply.sysex
        if reply.tag != MIDIControlCodec.tagUnknown && reply.tag != MIDIControlCodec.tagNotify {
            sysex[1 + reply.manufacturer.count + 4] = tag
        }
        return sysex
    }

    /// Run `call` (a request through the core) off main while every request
    /// it sends is answered with `reply` (re-tagged), or with `fallback`
    /// built by the Swift codec.  Returns the result and the requests sent.
    private func exchange<T>(_ r: Rig, reply: Vector?, fallback: [UInt8]? = nil,
                             _ call: @escaping () -> T) -> (T?, [(tag: UInt8, frame: FakeMIDIDevice.Request)]) {
        let start = r.port.device.requests.count
        var out: T?
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).async { out = call(); done.signal() }
        var answered = start
        let deadline = Date().addingTimeInterval(10)
        while done.wait(timeout: .now()) == .timedOut && Date() < deadline {
            let requests = r.port.device.requests
            while answered < requests.count {
                let tag = requests[answered].tag
                if let reply = reply {
                    r.port.device.sendRaw(Self.retag(reply, tag))
                } else if let payload = fallback {
                    r.port.device.sendRaw(MIDIControlCodec.encode(
                        frame: MIDIControlCodec.replyFrame(type: MIDIControlCodec.typeGetReply, status: 0, payload: payload),
                        tag: tag, manufacturer: MIDIControlCodec.developmentManufacturer))
                }
                answered += 1
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.005))
        }
        return (out, Array(r.port.device.requests.dropFirst(start)))
    }

    func testSW5TheCoreSpeaksTheFirmwareVectors() throws {
        let v = try vectors()
        XCTAssertGreaterThanOrEqual(v.count, 13)
        let r = rig()
        let source = try XCTUnwrap(r.transport.coreStateSource)
        r.port.device.respond = { _ in nil }   // from here on, the vectors answer
        var covered: Set<String> = []

        /// The core's request carries the vector's fields; wIndex is always
        /// the vendor interface (2) from the core, as on USB.
        func check(_ sent: [(tag: UInt8, frame: FakeMIDIDevice.Request)], against name: String,
                   file: StaticString = #filePath, line: UInt = #line) {
            guard let vec = v[name], let want = MIDIControlCodec.parseRequestFrame(vec.frame),
                  let got = sent.first?.frame else { return XCTFail("\(name): nothing sent", file: file, line: line) }
            XCTAssertTrue(got.crcOK, "\(name): CRC", file: file, line: line)
            XCTAssertEqual(got.type, want.type, "\(name): type", file: file, line: line)
            XCTAssertEqual(got.request, want.request, "\(name): bRequest", file: file, line: line)
            XCTAssertEqual(got.value, want.value, "\(name): wValue", file: file, line: line)
            XCTAssertEqual(got.payload, want.payload, "\(name): payload", file: file, line: line)
            XCTAssertEqual(got.index, 2, "\(name): wIndex", file: file, line: line)
            if want.index == 2 {
                // Same fields: the frame is the vector's, byte for byte.
                XCTAssertEqual(MIDIControlCodec.requestFrame(type: got.type, request: got.request, value: got.value,
                                                             index: got.index, length: got.length, payload: got.payload),
                               vec.frame, "\(name): frame", file: file, line: line)
            }
            covered.insert(name)
        }

        // GET_PLATFORM, answered by resp_platform.
        let (platform, sent1) = exchange(r, reply: v["resp_platform"]) {
            source.call { try $0.rawGet(opcode: 0x7F, value: 0, len: 7) }
        }
        check(sent1, against: "get_platform")
        XCTAssertEqual(try XCTUnwrap(platform).map { [UInt8]($0) }.sw5Value, [0x01, 0x01, 0x16, 0x04, 0x01, 0x06, 0x00])
        covered.insert("resp_platform")

        // SET_MASTER_VOLUME -20 dB, answered by resp_set_ok.
        let (setVolume, sent2) = exchange(r, reply: v["resp_set_ok"]) {
            source.call { try $0.rawSet(opcode: 0xD2, value: 0, data: Data([0x00, 0x00, 0xA0, 0xC1])) }
        }
        check(sent2, against: "set_master_volume")
        XCTAssertNotNil(try XCTUnwrap(setVolume).sw5Value, "resp_set_ok is a success")
        covered.insert("resp_set_ok")

        // GET_MASTER_VOLUME, answered by resp_get_master_volume (-20 dB).
        let (volume, _) = exchange(r, reply: v["resp_get_master_volume"]) {
            source.call { try $0.rawGet(opcode: 0xD3, value: 0, len: 4) }
        }
        XCTAssertEqual(try XCTUnwrap(volume).map { [UInt8]($0) }.sw5Value, [0x00, 0x00, 0xA0, 0xC1])
        covered.insert("resp_get_master_volume")

        // GET_STATUS 9 with a capped length: wIndex 2 in the vector too.
        let (_, sent3) = exchange(r, reply: nil, fallback: [UInt8](repeating: 0, count: 26)) {
            source.call { try $0.rawGet(opcode: 0x50, value: 9, len: 26) }
        }
        check(sent3, against: "get_status_9_capped")

        // A SET with wValue only, and one with 64 payload bytes.
        let (_, sent4) = exchange(r, reply: v["resp_set_ok"]) {
            source.call { try $0.rawSet(opcode: 0xD4, value: 0x0102, data: Data()) }
        }
        check(sent4, against: "set_wvalue_only")
        let (_, sent5) = exchange(r, reply: v["resp_set_ok"]) {
            source.call { try $0.rawSet(opcode: 0x20, value: 3, data: Data((0x80...0xBF).map { UInt8($0) })) }
        }
        check(sent5, against: "set_64_bytes")

        // BULK_LOCKED: the device did not act; the core retries, then says locked.
        let (locked, sent6) = exchange(r, reply: v["resp_bulk_locked"]) {
            source.call { try $0.rawSet(opcode: 0xD2, value: 0, data: Data([0x00, 0x00, 0xA0, 0xC1])) }
        }
        guard case .failure(.Locked)? = locked else { return XCTFail("resp_bulk_locked: \(String(describing: locked))") }
        XCTAssertGreaterThan(sent6.count, 1, "BULK_LOCKED is retried")
        covered.insert("resp_bulk_locked")

        // FRAME_ERROR with tag 0x7F: the answer to whatever is in flight.
        let (frameError, _) = exchange(r, reply: v["resp_frame_error_unknown_tag"]) {
            source.call { try $0.rawGet(opcode: 0xD3, value: 0, len: 4) }
        }
        if case .success? = frameError { XCTFail("a FRAME_ERROR answer is a failure") }
        XCTAssertNotNil(frameError, "answered at once, not by the timeout")
        covered.insert("resp_frame_error_unknown_tag")

        // A notification (tag 0): PARAM_CHANGED from MIDI, so our own.
        var notes: [LinkNotification] = []
        let token = r.transport.addNotificationObserver { notes.append($0) }
        let notify = try XCTUnwrap(v["notify_param_changed"])
        r.port.device.sendRaw(notify.sysex)
        let packet = Data(try XCTUnwrap(MIDIControlCodec.parseDeviceFrame(notify.frame)).payload)
        XCTAssertTrue(spinUntil(5) { notes.contains { $0.packet == packet } }, "the vector's notification reaches the observers")
        XCTAssertEqual(notes.first { $0.packet == packet }?.origin, r.transport.session)
        token.cancel()
        covered.insert("notify_param_changed")

        // Two the core's MIDI handle cannot send: a 3-byte manufacturer ID
        // (the handle uses the firmware's default 0x7D) and a raw 0xA1 (the
        // core's raw calls refuse the bulk opcodes).  dspi-proto's
        // midi_vectors test covers both byte for byte.
        let inRust: Set<String> = ["get_platform_3byte_mfr", "set_all_params_6136"]
        XCTAssertEqual(covered.union(inRust), Set(v.keys), "every vector is checked here or in Rust")
    }
}

private extension Result {
    /// The success, or nil.
    var sw5Value: Success? {
        if case .success(let s) = self { return s }
        return nil
    }
}
