import XCTest
import Combine
import dspi_core
@testable import DSPi_Console

/// WP SW4: the launch flag for the existing transport tests.  With
/// `TEST_RUNNER_DSPI_CORE_TRANSPORT=1`, `FakeTransportTests` and the extension
/// channel's tests drive the Rust core, which carries every request to the
/// same `FakeTransport` through CG26's host transport.
enum CoreTransportUnderTest {
    static var enabled: Bool { CoreBackedTransport.isEnabled }

    /// `fake` itself, or the core over it behind the flag.
    static func over(_ fake: FakeTransport) -> any DeviceTransport {
        enabled ? CoreBackedTransport(link: .host(fake, .fake)) : fake
    }

    /// The GETs the core's own connect sequence and `fetchAll` leave
    /// unanswered on a fresh fake: behind the flag they reach the fake too,
    /// and are not the console's reads.  Empty without the flag.
    static func connectSequenceGaps(extensions: Bool = true) -> Set<String> {
        guard enabled else { return [] }
        let fake = FakeTransport(autoSelect: false, extensions: extensions)
        let t = CoreBackedTransport(link: .host(fake, .fake))
        fake.selectDevice(FakeTransport.demoDevice)
        _ = t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7)
        // -- WP SW8 -- and its fetchAll, which the view model's read now is
        // (per-item reads such as the subharm solo, 0x2D).
        _ = t.readAll()
        return Set(fake.unansweredRequests)
    }
}

/// WP SW4: `CoreBackedTransport` on the core's demo device and over a Swift
/// transport (CG26), whatever the launch flag says.
final class CoreBackedTransportSW4Tests: XCTestCase {

    private func demo() -> CoreBackedTransport {
        let t = CoreBackedTransport(link: .demo)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(10) { t.isConnected }, "the core's demo device opens")
        return t
    }

    private func hosted(_ inner: any DeviceTransport, kind: CoreBackedTransport.HostKind = .fake,
                        select fake: FakeTransport) -> CoreBackedTransport {
        let t = CoreBackedTransport(link: .host(inner, kind))
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(10) { t.isConnected }, "the core opens over the host")
        return t
    }

    /// One GET on the queue, so every earlier SET has gone out.
    private func flush(_ t: any DeviceTransport) {
        _ = t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7)
    }

    private static func float(_ value: Float) -> Data { withUnsafeBytes(of: value) { Data($0) } }

    /// Passes only in the flagged run (ci-test.sh requires it there): the
    /// existing tests' transport really is the core.
    func testSW4LaunchFlagPutsTheCoreUnderTheTests() throws {
        guard CoreTransportUnderTest.enabled else { throw XCTSkip("DSPI_CORE_TRANSPORT is not 1") }
        let fake = FakeTransport(autoSelect: false)
        XCTAssertTrue(CoreTransportUnderTest.over(fake) is CoreBackedTransport)
    }

    // MARK: - The core's demo device

    func testSW4DemoDeviceAnswers() throws {
        let t = demo()
        XCTAssertEqual(t.availableDevices, [FakeTransport.demoDevice])
        XCTAssertEqual(t.selectedDevice, FakeTransport.demoDevice)
        XCTAssertNotEqual(t.generation, 0)
        let platform = try t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7).get()
        XCTAssertGreaterThanOrEqual(platform.count, 4)
        let bulk = try t.getControlResult(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                          length: UInt16(BULK_PARAMS_SIZE)).get()
        XCTAssertEqual(bulk.count, Int(BULK_PARAMS_SIZE))
        XCTAssertEqual(Int(bulk[0]), WIRE_FORMAT_VERSION)
    }

    func testSW4DemoWriteReadsBackAndEchoesAsOwn() throws {
        let t = demo()
        var notes: [LinkNotification] = []
        let token = t.addNotificationObserver { notes.append($0) }
        defer { token.cancel() }
        t.sendControlRequest(request: REQ_SET_PREAMP, value: 0, index: 2, data: Self.float(-3.5))
        let back = try t.getControlResult(request: REQ_GET_PREAMP, value: 0, index: 2, length: 4).get()
        XCTAssertEqual(back, Self.float(-3.5))
        XCTAssertTrue(spinUntil(5) { notes.contains { $0.eventID == 0x02 } }, "PARAM_CHANGED through the tap")
        let echo = try XCTUnwrap(notes.first { $0.eventID == 0x02 })
        XCTAssertEqual(echo.packet.first, 0x02)
        XCTAssertTrue(t.isOwnWrite(origin: echo.origin), "our own write's echo carries our session")
    }

    func testSW4DemoReconnectAndDisconnect() {
        let t = demo()
        let first = t.generation
        t.reconnect()
        XCTAssertNotEqual(t.generation, first, "a reopen is a new generation at once")
        XCTAssertTrue(spinUntil(10) { t.isConnected })
        let second = t.generation
        t.disconnect()
        XCTAssertNotEqual(t.generation, second)
        XCTAssertTrue(spinUntil(5) { !t.isConnected })
        XCTAssertNil(t.selectedDevice)
        XCTAssertEqual(t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
                       .failure(.noDevice))
    }

    // MARK: - Over a Swift transport (CG26)

    func testSW4HostFollowsTheFakesConnection() throws {
        let fake = FakeTransport(autoSelect: false)
        let t = CoreBackedTransport(link: .host(fake, .fake))
        XCTAssertFalse(t.isConnected)
        XCTAssertEqual(t.availableDevices, [FakeTransport.demoDevice], "the host's devices")
        XCTAssertEqual(t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
                       .failure(.noDevice))

        t.selectDevice(FakeTransport.demoDevice)            // selects on the host
        XCTAssertTrue(spinUntil(10) { t.isConnected })
        XCTAssertEqual(fake.selectedDevice, FakeTransport.demoDevice)
        let platform = try t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7).get()
        XCTAssertEqual([UInt8](platform), [1, 1, 0x16, 9, 1, 6, 5], "the Swift fake's capture")

        let before = t.generation
        fake.reconnect()
        XCTAssertNotEqual(t.generation, before)
        XCTAssertTrue(spinUntil(10) { t.isConnected })

        fake.disconnect()
        XCTAssertTrue(spinUntil(5) { !t.isConnected })
        XCTAssertEqual(t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
                       .failure(.noDevice))
    }

    func testSW4WritesReachTheHostInOrder() throws {
        let fake = FakeTransport(autoSelect: false)
        let t = hosted(fake, select: fake)
        for gain: Float in [1, 2, 3] {
            t.sendControlRequest(request: REQ_SET_OUTPUT_GAIN, value: 1, index: 2, data: Self.float(gain))
        }
        flush(t)
        let bulk = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                        length: UInt16(BULK_PARAMS_SIZE)))
        let at = BULK_OUTPUTS_OFFSET + 12 + 4
        XCTAssertEqual(bulk.subdata(in: at..<(at + 4)), Self.float(3), "the last write landed last")
    }

    func testSW4AStallIsAnErrorAndTheLinkStays() {
        let fake = FakeTransport(autoSelect: false, extensions: false)
        let t = hosted(fake, select: fake)
        XCTAssertEqual(t.getControlResult(request: REQ_EXTENSION, value: EXT_LIST, index: EXT_WINDEX,
                                          length: EXT_LIST_READ_LENGTH), .failure(.error))
        XCTAssertTrue(t.isConnected)
        XCTAssertNotNil(t.getControlRequest(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7))
    }

    /// Over a hub, the origin decides: our session's change is ours, a change
    /// at the device (origin 0) and another session's are not.
    func testSW4HubOriginsSetOwn() throws {
        let fake = FakeTransport(autoSelect: false)
        let inject = InjectingTransport(fake)
        let t = hosted(inject, kind: .hub, select: fake)
        XCTAssertEqual(t.session, fake.session)
        var notes: [LinkNotification] = []
        let token = t.addNotificationObserver { notes.append($0) }
        defer { token.cancel() }
        func paramChanged(seq: UInt8, source: UInt8) -> Data {
            // PARAM_CHANGED for the preamp, as the firmware sends it.
            var p = Data([0x02, 0x02, 0x00, seq, UInt8(BULK_GLOBAL_OFFSET & 0xFF), UInt8(BULK_GLOBAL_OFFSET >> 8),
                          4, 0, source, 0, 0, 0])
            p.append(Self.float(-1))
            return p
        }
        inject.inject(paramChanged(seq: 1, source: 1), origin: fake.session)
        inject.inject(paramChanged(seq: 2, source: 1), origin: 7)
        inject.inject(paramChanged(seq: 3, source: 0), origin: 0)
        XCTAssertTrue(spinUntil(5) { notes.count >= 3 })
        XCTAssertEqual(notes.map(\.origin), [t.session, 0, 0])
        XCTAssertEqual(notes.map { $0.packet[3] }, [1, 2, 3], "in order, verbatim")
    }

    func testSW4TheLockReachesTheHost() {
        let fake = FakeTransport(autoSelect: false)
        let locks = LockingTransport(fake)
        let t = hosted(locks, kind: .hub, select: fake)
        XCTAssertTrue(t.acquireDeviceLock(reason: "SW4", timeout: 5))
        XCTAssertEqual(locks.acquired, ["SW4"])
        t.releaseDeviceLock()
        flush(t)
        XCTAssertEqual(locks.released, 1)

        locks.refuse = true
        XCTAssertFalse(t.acquireDeviceLock(reason: "refused", timeout: 5), "another session holds it")
        XCTAssertNil(t.withDeviceLock(reason: "flow", timeout: 5) { 1 })
    }

    func testSW4ErrorsMapToTheStatusesCallersHandle() {
        let cases: [(DspiError, DSPi_Console.LinkStatus)] = [
            (.Disconnected, .noDevice), (.Locked, .locked), (.Denied, .denied), (.ReadOnly, .denied),
            (.Refused(status: 2), .error), (.InvalidArgument(msg: "x"), .error), (.Unsupported, .unsupported),
            (.Transfer(msg: "x"), .timeout), (.Link(code: "rate_limited", msg: "x"), .rateLimited),
            (.Link(code: "closed", msg: "x"), .noDevice),
        ]
        for (error, status) in cases {
            XCTAssertEqual(CoreBackedTransport.status(for: error), status, "\(error)")
        }
        // And a host's failure as the error the core expects (HostTransport).
        guard case .Disconnected = HostBridge.error(for: .noDevice, opcode: 0) else { return XCTFail("noDevice") }
        guard case .Refused(status: 2) = HostBridge.error(for: .error, opcode: 0) else { return XCTFail("a stall") }
        guard case .Locked = HostBridge.error(for: .locked, opcode: 0) else { return XCTFail("locked") }
        guard case .Transfer = HostBridge.error(for: .timeout, opcode: 0) else { return XCTFail("timeout") }
    }
}

// MARK: - Test transports

/// Lets a test push notifications with any origin, as a hub would.
final class InjectingTransport: ForwardingTransport {
    private let fanout = NotificationFanout()
    func inject(_ packet: Data, origin: LinkSessionID) {
        fanout.publish(LinkNotification(packet: packet, origin: origin, receivedAt: Date()))
    }
    override func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        fanout.add(handler)
    }
}

/// A hub's lock: records what the core asks for, and refuses on request.
final class LockingTransport: ForwardingTransport {
    private let lock = NSLock()
    private var _acquired: [String] = []
    private var _released = 0
    var refuse = false
    var acquired: [String] { lock.lock(); defer { lock.unlock() }; return _acquired }
    var released: Int { lock.lock(); defer { lock.unlock() }; return _released }

    override func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard !refuse else { return false }
        _acquired.append(reason)
        return true
    }
    override func releaseDeviceLock() { lock.lock(); _released += 1; lock.unlock() }
}
