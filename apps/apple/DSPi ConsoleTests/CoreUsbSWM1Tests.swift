//
//  CoreUsbSWM1Tests.swift
//  DSPi ConsoleTests
//
//  WP SW-M1: the Mac's USB through the core (Core/CoreUsb.swift), on a
//  scripted bus.  No device: each "DSPi" on the bus is Swift's FakeTransport
//  behind a mock of the core's UsbPipe, so the whole local stack runs as the
//  app builds it: the core (CoreBackedTransport) over the hub's session 1
//  (HubTransport, LinkHub) over CoreUsb.
//

#if os(macOS)
import XCTest
import Combine
import dspi_core
@testable import DSPi_Console

/// A DSPi on the scripted bus: transfers go to a FakeTransport, its
/// notifications come back through `readNotification`.
final class MockUsbPipe: UsbPipeProtocol, @unchecked Sendable {
    let serialValue: String
    let fake: FakeTransport
    private let lock = NSCondition()
    private var queue: [Data] = []
    private var closed = false
    private var streamEnded = false
    private var token: AnyCancellable?
    /// Opcodes answered with a scripted error instead of the fake.
    var failures: [UInt8: DspiError] = [:]
    private(set) var outRequests: [UInt8] = []

    init(serial: String) {
        serialValue = serial
        fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        token = fake.addNotificationObserver { [weak self] note in self?.push(note.packet) }
    }

    func push(_ packet: Data) {
        lock.lock(); queue.append(packet); lock.signal(); lock.unlock()
    }

    /// The device went away: the stream ends.
    func endStream() {
        lock.lock(); streamEnded = true; lock.broadcast(); lock.unlock()
    }

    var isClosedNow: Bool { lock.lock(); defer { lock.unlock() }; return closed }

    func close() {
        lock.lock(); closed = true; lock.broadcast(); lock.unlock()
    }

    func isClosed() -> Bool { isClosedNow }

    func serial() -> String { serialValue }

    func controlIn(opcode: UInt8, value: UInt16, len: UInt16) throws -> Data {
        if isClosedNow { throw DspiError.Disconnected }
        if let e = failures[opcode] { throw e }
        switch fake.getControlResult(request: opcode, value: value, index: 2, length: len) {
        case .success(let d): return d
        case .failure(.noDevice): throw DspiError.Disconnected
        case .failure: throw DspiError.Refused(status: 0)
        }
    }

    func controlOut(opcode: UInt8, value: UInt16, data: Data) throws {
        if isClosedNow { throw DspiError.Disconnected }
        if let e = failures[opcode] { throw e }
        lock.lock(); outRequests.append(opcode); lock.unlock()
        fake.sendControlRequest(request: opcode, value: value, index: 2, data: data)
    }

    func readNotification(timeoutMs: UInt32) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        let deadline = Date().addingTimeInterval(Double(timeoutMs) / 1000)
        while queue.isEmpty && !closed && !streamEnded {
            if !lock.wait(until: deadline) { return Data() }
        }
        if closed || streamEnded { throw DspiError.Disconnected }
        return queue.removeFirst()
    }
}

/// The scripted bus: which DSPis are plugged in, and how opens go.
final class MockUsbBus: CoreUsbBus {
    private let lock = NSLock()
    private var present: [DSPiDevice] = []
    private(set) var pipes: [String: [MockUsbPipe]] = [:]
    /// Opens of this serial that fail first (busy), before one works.
    var busyOpens: [String: Int] = [:]
    var alwaysBusy: Set<String> = []

    func plug(_ serial: String, location: UInt32 = 0x1400_0000) {
        lock.lock(); present.append(DSPiDevice(serial: serial, locationID: location)); lock.unlock()
    }

    func unplug(_ serial: String) {
        lock.lock(); present.removeAll { $0.serial == serial }; lock.unlock()
        pipes[serial]?.last?.endStream()
    }

    func scan() -> [DSPiDevice] {
        lock.lock(); defer { lock.unlock() }; return present
    }

    func open(serial: String) throws -> UsbPipeProtocol {
        lock.lock(); defer { lock.unlock() }
        guard present.contains(where: { $0.serial == serial }) else { throw DspiError.Disconnected }
        if alwaysBusy.contains(serial) {
            throw DspiError.Transfer(msg: "could not open interface for exclusive access")
        }
        if let n = busyOpens[serial], n > 0 {
            busyOpens[serial] = n - 1
            throw DspiError.Transfer(msg: "busy")
        }
        let p = MockUsbPipe(serial: serial)
        pipes[serial, default: []].append(p)
        return p
    }

    func opens(_ serial: String) -> Int { lock.lock(); defer { lock.unlock() }; return pipes[serial]?.count ?? 0 }
    func lastPipe(_ serial: String) -> MockUsbPipe? { lock.lock(); defer { lock.unlock() }; return pipes[serial]?.last }
}

final class CoreUsbSWM1Tests: XCTestCase {

    private func usb(_ bus: MockUsbBus) -> CoreUsb {
        CoreUsb(startMonitoring: true, bus: bus, pollInterval: 0.05)
    }

    // MARK: Hot-plug, selection, auto-connect

    func testSWM1APluggedDSPiIsListedAndOpened() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected }, "the first DSPi on the bus opens by itself")
        XCTAssertEqual(usb.availableDevices.map(\.serial), ["SWM1-A"])
        XCTAssertEqual(usb.selectedDevice?.serial, "SWM1-A")
        XCTAssertNil(usb.errorMessage)
        XCTAssertEqual(bus.opens("SWM1-A"), 1)
    }

    func testSWM1UnplugMovesToTheOtherDSPiAndReplugReconnects() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected && usb.selectedDevice?.serial == "SWM1-A" })
        bus.plug("SWM1-B")
        XCTAssertTrue(spinUntil(5) { usb.availableDevices.count == 2 })
        XCTAssertEqual(usb.selectedDevice?.serial, "SWM1-A", "a second DSPi does not take over")

        bus.unplug("SWM1-A")
        XCTAssertTrue(spinUntil(5) { usb.selectedDevice?.serial == "SWM1-B" && usb.isConnected },
                      "the survivor is selected")
        XCTAssertTrue(bus.lastPipe("SWM1-A")?.isClosedNow ?? false, "the departed device is closed")
        XCTAssertEqual(usb.availableDevices.map(\.serial), ["SWM1-B"])

        bus.unplug("SWM1-B")
        XCTAssertTrue(spinUntil(5) { usb.availableDevices.isEmpty && !usb.isConnected })
        XCTAssertNil(usb.selectedDevice)

        bus.plug("SWM1-B")
        XCTAssertTrue(spinUntil(5) { usb.isConnected && usb.selectedDevice?.serial == "SWM1-B" },
                      "a DSPi plugged into an empty bus opens")
    }

    func testSWM1ReplugOfTheSelectedDSPiReconnectsIt() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        let generation = usb.generation
        bus.unplug("SWM1-A")
        XCTAssertTrue(spinUntil(5) { !usb.isConnected })
        XCTAssertEqual(usb.errorMessage, "Device Removed")
        XCTAssertEqual(usb.selectedDevice?.serial, "SWM1-A", "the selection is kept for the replug")
        bus.plug("SWM1-A")
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        XCTAssertEqual(bus.opens("SWM1-A"), 2)
        XCTAssertNotEqual(usb.generation, generation)
        XCTAssertNil(usb.errorMessage)
    }

    /// A replug faster than one poll: the bus lists the serial throughout,
    /// but the old pipe's stream ended.  The device is opened again.
    func testSWM1AStreamThatEndsReopensADeviceStillListed() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        bus.lastPipe("SWM1-A")?.endStream()
        XCTAssertTrue(spinUntil(5) { bus.opens("SWM1-A") == 2 && usb.isConnected })
        XCTAssertTrue(bus.pipes["SWM1-A"]?.first?.isClosedNow ?? false)
    }

    func testSWM1ABusyOpenIsRetriedThenReported() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        bus.busyOpens["SWM1-A"] = 2
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected }, "the third try opens it")

        let bus2 = MockUsbBus()
        bus2.plug("SWM1-B")
        bus2.alwaysBusy = ["SWM1-B"]
        let usb2 = self.usb(bus2)
        XCTAssertTrue(spinUntil(8) { usb2.errorMessage != nil })
        XCTAssertEqual(usb2.errorMessage, "Device busy (another app has it open)")
        XCTAssertFalse(usb2.isConnected)
    }

    /// SWF2's F2 case, now that USB is the core's: nusb claims only the
    /// vendor interface, so a DSPi whose MIDI port CoreMIDI holds opens without
    /// IOKit's exclusive device open (the old OpenPlan.shareDevice).  What
    /// stays is the retry and the wording of an open that keeps failing.
    func testSWM1AHeldDeviceOpensAfterTheRetriesOrSaysBusy() {
        XCTAssertEqual(CoreUsb.openRetryDelays, [0.1, 0.2, 0.4, 0.8, 1.0])
        XCTAssertEqual(CoreUsb.openFailureMessage(DspiError.Transfer(msg: "could not open device for exclusive access")),
                       "Device busy (another app has it open)")
        XCTAssertEqual(CoreUsb.openFailureMessage(DspiError.Transfer(msg: "usb error: other")), "Could not open device")
    }

    func testSWM1DisconnectStaysClosedUntilReconnect() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        usb.disconnect()
        XCTAssertTrue(spinUntil(2) { !usb.isConnected })
        RunLoop.main.run(until: Date().addingTimeInterval(0.3))
        XCTAssertFalse(usb.isConnected, "an explicit disconnect is not undone by the poll")
        XCTAssertEqual(usb.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
                       .failure(.noDevice))
        usb.reconnect()
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
    }

    // MARK: Transfers

    func testSWM1TransfersPassThroughAndFailuresMapToLinkStatuses() throws {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        let platform = try XCTUnwrap(usb.getControlRequest(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7))
        XCTAssertNotNil(FirmwareVersion.fromPlatformReply([UInt8](platform)))
        XCTAssertEqual(usb.sendControlResult(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0,
                                             data: withUnsafeBytes(of: Float(-20)) { Data($0) }), .ok)
        XCTAssertEqual(bus.lastPipe("SWM1-A")?.outRequests, [REQ_SET_MASTER_VOLUME])

        let pipe = try XCTUnwrap(bus.lastPipe("SWM1-A"))
        pipe.failures[REQ_GET_SERIAL] = .Refused(status: 0)
        pipe.failures[REQ_GET_DEVICE_NAME] = .Link(code: "timeout", msg: "timed out")
        pipe.failures[REQ_GET_PLATFORM] = .Disconnected
        pipe.failures[REQ_SET_MASTER_VOLUME] = .Transfer(msg: "usb error")
        XCTAssertEqual(usb.getControlResult(request: REQ_GET_SERIAL, value: 0, index: 2, length: 16), .failure(.error),
                       "a stall is an ERROR, not a dead link")
        XCTAssertEqual(usb.getControlResult(request: REQ_GET_DEVICE_NAME, value: 0, index: 2, length: 32),
                       .failure(.timeout))
        XCTAssertEqual(usb.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
                       .failure(.noDevice))
        XCTAssertEqual(usb.sendControlResult(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0, data: Data(count: 4)),
                       .error)
    }

    func testSWM1SelectingAnotherDSPiClosesTheFirstAndMovesTheGeneration() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        bus.plug("SWM1-B")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        let first = usb.selectedDevice!.serial
        let other = first == "SWM1-A" ? "SWM1-B" : "SWM1-A"
        let firstGeneration = usb.generation
        usb.selectDevice(DSPiDevice(serial: other, locationID: 0))
        XCTAssertTrue(spinUntil(5) { usb.selectedDevice?.serial == other && usb.isConnected })
        XCTAssertGreaterThan(usb.generation, firstGeneration)
        XCTAssertTrue(bus.lastPipe(first)?.isClosedNow ?? false, "the first device is released")
        XCTAssertEqual(usb.sendControlResult(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0,
                                             data: withUnsafeBytes(of: Float(-3)) { Data($0) }), .ok)
        XCTAssertEqual(bus.lastPipe(other)?.outRequests, [REQ_SET_MASTER_VOLUME], "work goes to the open one")
        XCTAssertEqual(bus.lastPipe(first)?.outRequests, [])
    }

    // MARK: Notifications

    func testSWM1NotificationsReachTheFanoutWithUSBsAttribution() {
        let bus = MockUsbBus()
        bus.plug("SWM1-A")
        let usb = usb(bus)
        XCTAssertTrue(spinUntil(5) { usb.isConnected })
        var notes: [LinkNotification] = []
        let token = usb.addNotificationObserver { notes.append($0) }
        defer { token.cancel() }
        let pipe = bus.lastPipe("SWM1-A")!
        func paramChanged(seq: UInt8, source: UInt8) -> Data {
            var p = Data([0x02, 0x02, 0x00, seq, UInt8(BULK_GLOBAL_OFFSET & 0xFF), UInt8(BULK_GLOBAL_OFFSET >> 8),
                          4, 0, source, 0, 0, 0])
            p.append(withUnsafeBytes(of: Float(-1)) { Data($0) })
            return p
        }
        pipe.push(Data([0x00]))                                  // idle keep-alive: dropped
        pipe.push(Data([0x01, 0x01, 0, 0, 0, 0, 0, 0]))          // v1: dropped
        pipe.push(paramChanged(seq: 1, source: 1))               // HOST_SET: ours (session 1)
        pipe.push(paramChanged(seq: 2, source: 4))               // another source: the device's
        XCTAssertTrue(spinUntil(5) { notes.count >= 2 })
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        XCTAssertEqual(notes.map { $0.packet[3] }, [1, 2], "v2 only, in order, verbatim")
        XCTAssertEqual(notes.map(\.origin), [usb.session, 0])
    }

    // MARK: The default stack: the core over the hub over CoreUsb

    /// The app's macOS default (AppState): CoreBackedTransport over
    /// HubTransport (the hub's session 1) over LinkHub over CoreUsb.  A
    /// plugged DSPi is opened, the hub attaches it, the core connects through
    /// it and reads it, a typed write reaches the device, and an unplug and
    /// a replug follow through every layer.
    func testSWM1TheCoreDrivesAPluggedDSPiThroughTheHub() throws {
        let bus = MockUsbBus()
        let usb = usb(bus)
        let auth = LinkAuthStore(storeURL: FileManager.default.temporaryDirectory
            .appendingPathComponent("swm1-\(UUID().uuidString).json"))
        let hub = LinkHub(usb: usb, policy: LinkPolicy.bundled ?? LinkPolicy.empty, auth: auth)
        let local = HubTransport(hub: hub, usb: usb)
        let core = CoreBackedTransport(link: .host(local, .hub))
        XCTAssertFalse(core.isConnected)

        bus.plug("SWM1-A")
        XCTAssertTrue(spinUntil(15) { core.isConnected }, "the core connects through the hub")
        XCTAssertEqual(core.selectedDevice?.serial, "SWM1-A")
        let source = try XCTUnwrap(core.coreStateSource, "typed features are on: the core is attached")
        XCTAssertGreaterThan(source.readState().numOutputChannels, 0, "the core read the device's image")

        let pipe = try XCTUnwrap(bus.lastPipe("SWM1-A"))
        source.write(lane: nil) { try $0.setMasterVolume(db: -17) }
        XCTAssertTrue(spinUntil(5) { pipe.outRequests.contains(REQ_SET_MASTER_VOLUME) },
                      "a typed write goes out through the hub and the pipe")

        bus.unplug("SWM1-A")
        XCTAssertTrue(spinUntil(10) { !core.isConnected }, "the core follows the unplug")
        bus.plug("SWM1-A")
        XCTAssertTrue(spinUntil(15) { core.isConnected }, "and the replug")
        XCTAssertEqual(bus.opens("SWM1-A"), 2)
        withExtendedLifetime((hub, local)) {}
    }
}
/// WP SW-M1 on a real DSPi (TEST_RUNNER_DSPI_USB_HW=1; skipped otherwise):
/// the Mac's default stack, as a plain launch builds it (AppState: the core
/// over the hub's session 1 over CoreUsb).  RAM only: nothing is saved to
/// flash, no voltage write, no audio.  `HardwareTest.requireDevice` takes the
/// raw 0xA0 image first and fails the test if it differs at the end, so every
/// write here is put back.
final class CoreUsbSWM1HardwareTests: XCTestCase {

    private func masterVolume(_ usb: CoreUsb) -> Float? {
        HardwareTest.readFloat(usb, REQ_GET_MASTER_VOLUME, value: 0)
    }

    private func waitFor(_ usb: CoreUsb, volume: Float, _ what: String) {
        XCTAssertTrue(spinUntil(5) { (self.masterVolume(usb)).map { abs($0 - volume) < 0.26 } ?? false },
                      "\(what): the device reads \(masterVolume(usb).map { "\($0)" } ?? "nothing"), not \(volume)")
    }

    func testSWM1HardwareTheDefaultStackReadsWritesUndoesRecallsAndLocks() throws {
        let usb = try HardwareTest.requireDevice(self)
        XCTAssertTrue(AppState.usesCoreTransport, "a plain launch is on the core")
        let transport = AppState.shared.transport
        let source = try XCTUnwrap(transport.coreStateSource, "the core is attached on a plain launch")
        print("SWM1-HW device \(usb.selectedDevice?.serial ?? "?")")

        // Read: the core's mirror is the device's image.
        let state = source.readState()
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        let before = try XCTUnwrap(masterVolume(usb), "the master volume reads over USB")
        XCTAssertEqual(state.masterVolumeDb, before, accuracy: 0.26, "the core's mirror matches the device")
        print("SWM1-HW master volume before \(before) dB, outputs \(state.numOutputChannels)")

        // A snapshot of the device as found, in a scratch folder.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("swm1-snap-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        _ = try source.call { try $0.snapshotSetDir(dir: dir.path) }.get()
        let snap = try source.call { try $0.snapshotCapture(name: "SW-M1 before", note: "") }.get()

        // Writes through the typed API, through the hub and the core's USB.
        let a = max(before - 6, -60), b = max(before - 12, -60)
        source.write(lane: nil) { try $0.setMasterVolume(db: a) }
        waitFor(usb, volume: a, "first write")
        source.write(lane: nil) { try $0.setMasterVolume(db: b) }
        waitFor(usb, volume: b, "second write")

        // Undo and redo, from the core's journal.
        XCTAssertTrue(spinUntil(5) { source.readJournal().canUndo })
        _ = source.stepJournal(redo: false)
        waitFor(usb, volume: a, "undo")
        _ = source.stepJournal(redo: true)
        waitFor(usb, volume: b, "redo")

        // Snapshot recall puts the device back as found.
        let recall = try source.call { try $0.snapshotRecall(id: snap.id) }.get()
        print("SWM1-HW recall: \(recall.changes.count) changes, image \(recall.image)")
        waitFor(usb, volume: before, "snapshot recall")
        _ = try source.call { try $0.snapshotDelete(id: snap.id) }.get()

        // The lock: held by this console (the hub's session 1), another
        // session's write is refused.  The probe writes the current value.
        let hub = AppState.shared.linkHub
        XCTAssertTrue(transport.acquireDeviceLock(reason: "SW-M1 hardware check", timeout: 5))
        XCTAssertEqual(hub.registry.device(handle: hub.currentHandle)?.lockedBy, AppState.shared.localTransport.session)
        let remote = hub.openSession(role: .control)
        defer { hub.closeSession(remote.id) }
        let probe = LinkCmdRequest(tag: 1, handle: hub.currentHandle, direction: .set, bRequest: REQ_SET_MASTER_VOLUME,
                                   wValue: 0, wIndex: 2, payload: withUnsafeBytes(of: before) { Data($0) })
        let refused = expectation(description: "the other session's write is answered")
        var status: DSPi_Console.LinkStatus?
        hub.submit(probe, from: remote.id) { status = $0.status; refused.fulfill() }
        wait(for: [refused], timeout: 5)
        XCTAssertEqual(status, .locked, "another session cannot write while the console holds the lock")
        transport.releaseDeviceLock()
        XCTAssertTrue(spinUntil(5) { hub.registry.device(handle: hub.currentHandle)?.lockedBy == nil })
        waitFor(usb, volume: before, "the end")
    }

    /// Run alone, with the host unplugging the DSPi (QMP device_del) and
    /// plugging it back (device_add) once this test has started.
    func testSWM1HardwareUnplugAndReplug() throws {
        guard ProcessInfo.processInfo.environment["DSPI_SWM1_HOTPLUG"] == "1" else {
            throw XCTSkip("TEST_RUNNER_DSPI_SWM1_HOTPLUG=1: the host unplugs and replugs the DSPi")
        }
        let usb = try HardwareTest.requireDevice(self, timeout: 60)
        let transport = AppState.shared.transport
        let serial = try XCTUnwrap(usb.selectedDevice?.serial)
        print("SWM1-HW hotplug: waiting for the unplug of \(serial)")
        XCTAssertTrue(spinUntil(180) { !usb.isConnected && !transport.isConnected }, "the unplug is seen")
        XCTAssertEqual(usb.errorMessage, "Device Removed")
        // The notification stream ends first (CoreUsb closes at once); the
        // bus poll drops the device from the list a moment later.
        XCTAssertTrue(spinUntil(5) { !usb.availableDevices.contains { $0.serial == serial } },
                      "the departed DSPi leaves the list")
        print("SWM1-HW hotplug: unplugged, waiting for the replug")
        XCTAssertTrue(spinUntil(120) { usb.isConnected && transport.isConnected
                          && transport.coreStateSource != nil }, "the replug reconnects through every layer")
        XCTAssertEqual(usb.selectedDevice?.serial, serial)
        XCTAssertNotNil(HardwareTest.readFloat(usb, REQ_GET_MASTER_VOLUME, value: 0), "and the device answers")
        XCTAssertTrue(HardwareTest.waitForFullRead(AppState.shared.viewModel, timeout: 30), "the app read it again")
        print("SWM1-HW hotplug: reconnected")
    }
}
#endif
