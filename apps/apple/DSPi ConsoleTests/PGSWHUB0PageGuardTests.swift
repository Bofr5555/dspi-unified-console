//
//  PGSWHUB0PageGuardTests.swift
//  DSPi ConsoleTests
//
//  WP PGSWHUB0 (PLAN-PG 1.5): the Swift hub (the -CoreHub NO / -CoreTransport
//  NO paths) refuses a request on a page of upstream's paged protocol, a
//  non-zero wIndex high byte, with 0x86 UNSUPPORTED for commands and polls,
//  and sends nothing to USB: every device it attaches is legacy, and legacy
//  firmware would run the 1.1.6 opcode with the same number (GET 0x0553 is
//  REQ_FACTORY_RESET 0x53 there).  Page 0 (wIndex 0, the Mac's global index,
//  and 2, the vendor interface) passes.  No device: the hub's router and poll
//  scheduler over USBHubDevice over CoreUsb on a scripted bus whose DSPi is
//  Swift's FakeTransport, every transfer counted.
//

#if os(macOS)
import XCTest
import dspi_core
@testable import DSPi_Console

/// The app's Link status (dspi_core exports a type of the same name).
private typealias HubStatus = DSPi_Console.LinkStatus

/// A DSPi on the scripted bus that records every control transfer and hands
/// it to a FakeTransport.
private final class PGSWHUB0CountingPipe: UsbPipeProtocol, @unchecked Sendable {
    let fake: FakeTransport
    private let lock = NSCondition()
    private var calls: [UInt8] = []
    private var closed = false

    init() {
        fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
    }

    /// Every opcode that reached the "wire", in order.
    var transfers: [UInt8] { lock.lock(); defer { lock.unlock() }; return calls }

    func controlIn(opcode: UInt8, value: UInt16, len: UInt16) throws -> Data {
        lock.lock(); calls.append(opcode); lock.unlock()
        switch fake.getControlResult(request: opcode, value: value, index: 2, length: len) {
        case .success(let d): return d
        case .failure: throw DspiError.Refused(status: 0)
        }
    }

    func controlOut(opcode: UInt8, value: UInt16, data: Data) throws {
        lock.lock(); calls.append(opcode); lock.unlock()
        fake.sendControlRequest(request: opcode, value: value, index: 2, data: data)
    }

    func shutdown() { lock.lock(); closed = true; lock.broadcast(); lock.unlock() }
    func isShutDown() -> Bool { lock.lock(); defer { lock.unlock() }; return closed }
    func serial() -> String { "PGSWHUB0" }

    func readNotification(timeoutMs: UInt32) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        _ = lock.wait(until: Date().addingTimeInterval(Double(timeoutMs) / 1000))
        if closed { throw DspiError.Disconnected }
        return Data()
    }
}

private final class PGSWHUB0Bus: CoreUsbBus {
    let pipe = PGSWHUB0CountingPipe()
    func scan() -> [DSPiDevice] { [DSPiDevice(serial: "PGSWHUB0", locationID: 0x1400_0000)] }
    func open(serial: String) throws -> UsbPipeProtocol { pipe }
    func waitForChange(after seen: UInt64, timeoutMs: UInt32) -> UInt64 {
        Thread.sleep(forTimeInterval: Double(timeoutMs) / 1000)
        return seen
    }
}

final class PGSWHUB0PageGuardTests: XCTestCase {

    func testPGSWHUB0APageIsRefusedWithUnsupportedAndNeverReachesUSB() {
        let bus = PGSWHUB0Bus()
        let usb = CoreUsb(startMonitoring: true, bus: bus, pollInterval: 0.05)
        XCTAssertTrue(spinUntil(5) { usb.isConnected }, "the scripted DSPi opens")
        let info = HubDeviceInfo(serial: "PGSWHUB0", platform: 1, firmware: "1.1.6",
                                 outputs: 9, inputs: nil, wireVersion: nil, link: .usb)
        let device = USBHubDevice(usb: usb, info: info)
        let policy = LinkPolicy.bundled ?? .empty
        let router = CommandRouter(handle: 0, device: device, policy: policy)
        let scheduler = PollScheduler(device: device, policy: policy, driveWithTimer: false)
        defer { scheduler.stop() }
        let admin = RouterSession(id: 2, role: .admin)

        func get(_ code: UInt8, index: UInt16) -> HubStatus {
            let done = expectation(description: "routed \(code) \(index)")
            var status: HubStatus = .ok
            let request = LinkCmdRequest(tag: 7, handle: 0, direction: .get, bRequest: code,
                                         wValue: 0, wIndex: index, wLength: 4)
            router.submit(request, session: admin) { status = $0.response.status; done.fulfill() }
            wait(for: [done], timeout: 5)
            return status
        }
        func set(_ code: UInt8, index: UInt16) -> HubStatus {
            let done = expectation(description: "routed set \(code) \(index)")
            var status: HubStatus = .ok
            let request = LinkCmdRequest(tag: 8, handle: 0, direction: .set, bRequest: code,
                                         wValue: 0, wIndex: index, payload: Data([0]))
            router.submit(request, session: admin) { status = $0.response.status; done.fulfill() }
            wait(for: [done], timeout: 5)
            return status
        }

        let before = bus.pipe.transfers.count

        // Commands: GET 0x0553 (paged; REQ_FACTORY_RESET on 1.1.6), the
        // dangerous reads 0x0892/0x0891/0x0890, the SET 0x0844, and a page
        // the policy has no row for: UNSUPPORTED, before the policy.
        XCTAssertEqual(get(0x53, index: 0x0502), .unsupported, "GET 0x0553")
        for (code, index) in [(0x92, 0x0802), (0x91, 0x0802), (0x90, 0x0802), (0x53, 0xFFFF)] as [(UInt8, UInt16)] {
            XCTAssertEqual(get(code, index: index), .unsupported, "GET \(code) wIndex \(index)")
        }
        XCTAssertEqual(set(0x44, index: 0x0802), .unsupported, "SET 0x0844")
        // The device itself refuses too, for every other caller of execute.
        XCTAssertEqual(device.execute(LinkCmdRequest(tag: 1, handle: 0, direction: .get, bRequest: 0x53,
                                                     wValue: 0, wIndex: 0x0500, wLength: 4)).status,
                       .unsupported)

        // A poll on page 0x05: granted 0, reported UNSUPPORTED, never polled.
        var pollErrors: [(Int, HubStatus)] = []
        scheduler.onPollError = { _, slot, _, status in pollErrors.append((slot, status)) }
        let paged = PollScheduler.PollSpec(handle: 0, req: REQ_GET_STATUS, wValue: 0, wIndex: 0x0502, len: 4)
        let refused = scheduler.subscribe(session: 2, role: .admin, requests: [(slot: 0, spec: paged, hz: 20)])
        XCTAssertEqual(refused.map { $0.grantedHz }, [0])
        XCTAssertEqual(pollErrors.map { $0.0 }, [0])
        XCTAssertEqual(pollErrors.map { $0.1 }, [HubStatus.unsupported])
        XCTAssertEqual(scheduler.activeSpecCount, 0)
        for _ in 0..<40 { scheduler.tick() }

        XCTAssertEqual(bus.pipe.transfers.count, before, "no USB transfer for any paged request")

        // Page 0 passes: wIndex 0 (the Mac's global index) and 2.
        XCTAssertNotEqual(get(0x53, index: 0), .unsupported)
        XCTAssertNotEqual(get(0x53, index: 2), .unsupported)
        XCTAssertEqual(bus.pipe.transfers.suffix(2), [0x53, 0x53], "page 0 reaches the device")

        for (slot, index) in [(1, 0), (2, 2)] as [(Int, UInt16)] {
            let spec = PollScheduler.PollSpec(handle: 0, req: REQ_GET_STATUS, wValue: 0, wIndex: index, len: 4)
            let granted = scheduler.subscribe(session: 2, role: .admin, requests: [(slot: slot, spec: spec, hz: 20)])
            XCTAssertGreaterThan(granted.first?.grantedHz ?? 0, 0, "poll wIndex \(index)")
        }
        XCTAssertEqual(scheduler.activeSpecCount, 2)
        let polledBefore = bus.pipe.transfers.filter { $0 == REQ_GET_STATUS }.count
        for _ in 0..<40 { scheduler.tick() }
        XCTAssertGreaterThan(bus.pipe.transfers.filter { $0 == REQ_GET_STATUS }.count, polledBefore,
                             "page-0 polls reach the device")
        XCTAssertEqual(pollErrors.count, 1, "no further refusals")
        withExtendedLifetime(device) {}
    }
}
#endif
