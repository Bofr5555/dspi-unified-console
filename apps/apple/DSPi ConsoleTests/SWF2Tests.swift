//
//  SWF2Tests.swift
//  DSPi ConsoleTests
//
//  WP SWF2, from the Mac app's hardware validation (hw-mac):
//  - F2: USB does not depend on the DSPi's MIDI port, and a local device that
//    cannot be opened on its vendor interface falls back to its MIDI port;
//  - F3: a write the hub refuses is put back on screen, with the wording;
//  - F4: the Swift hub refuses core voltage raises from network sessions, as
//    the Rust hub's HB2 does; the app's own session is exempt.
//  No device: mocks, the Link client fake and the demo device.
//

import XCTest
import Combine
@testable import DSPi_Console

// MARK: - F2: the local set falls back to MIDI

/// A local member whose open succeeds or fails on demand.
private final class ScriptedMember: DeviceTransport {
    let devices: CurrentValueSubject<[DSPiDevice], Never>
    let selected = CurrentValueSubject<DSPiDevice?, Never>(nil)
    let connected = CurrentValueSubject<Bool, Never>(false)
    let error = CurrentValueSubject<String?, Never>(nil)
    var opens: Bool
    private(set) var selectCalls: [DSPiDevice] = []
    private(set) var sets: [UInt8] = []
    private(set) var disconnects = 0
    private(set) var lockCalls = 0
    private let fanout = NotificationFanout()

    init(_ devices: [DSPiDevice], opens: Bool) {
        self.devices = CurrentValueSubject(devices)
        self.opens = opens
    }

    /// The member's own open attempt (an auto-connect) failed.
    func failOpen() {
        connected.send(false)
        error.send("Could not open device")
    }

    /// The member opened its device by itself.
    func openByItself(_ device: DSPiDevice) {
        selected.send(device)
        connected.send(true)
        error.send(nil)
    }

    var session: LinkSessionID { 1 }
    var generation: UInt64 { 0 }
    var isConnected: Bool { connected.value }
    var isConnectedPublisher: AnyPublisher<Bool, Never> { connected.eraseToAnyPublisher() }
    var availableDevices: [DSPiDevice] { devices.value }
    var availableDevicesPublisher: AnyPublisher<[DSPiDevice], Never> { devices.eraseToAnyPublisher() }
    var selectedDevice: DSPiDevice? { selected.value }
    var selectedDevicePublisher: AnyPublisher<DSPiDevice?, Never> { selected.eraseToAnyPublisher() }
    var errorMessage: String? { error.value }
    var errorMessagePublisher: AnyPublisher<String?, Never> { error.eraseToAnyPublisher() }

    func selectDevice(_ device: DSPiDevice) {
        selectCalls.append(device)
        if opens { openByItself(device) } else { failOpen() }
    }
    func reconnect() {}
    func disconnect() { disconnects += 1; selected.send(nil); connected.send(false) }
    func markDisconnected() { connected.send(false) }
    func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) { sets.append(request) }
    func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, LinkStatus> {
        .failure(.noDevice)
    }
    func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        fanout.add(handler)
    }
    func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool { lockCalls += 1; return false }
    func releaseDeviceLock() {}
}

final class SWF2LocalFallbackTests: XCTestCase {
    private let usbDevice = DSPiDevice(serial: "BB66A44F2AC1E7B6", locationID: 0x0110_0000)
    private let midiDevice = DSPiDevice(serial: "BB66A44F2AC1E7B6", locationID: 0x0110_0000, hub: nil,
                                        remoteName: "DSPi (2AC1E7B6) via USB MIDI")

    func testSWF2AVendorPathThatCannotOpenFallsBackToMIDI() {
        let usb = ScriptedMember([usbDevice], opens: false)
        let midi = ScriptedMember([midiDevice], opens: true)
        let set = LocalTransportSet([usb, midi])
        XCTAssertEqual(set.availableDevices.count, 1, "one DSPi, one row, on both paths")
        XCTAssertNil(set.availableDevices.first?.remoteName, "the vendor path's row")

        usb.failOpen()
        XCTAssertEqual(midi.selectCalls, [midiDevice], "the MIDI member opens the same DSPi")
        XCTAssertTrue(set.isConnected)
        XCTAssertEqual(set.selectedDevice?.remoteName, "DSPi (2AC1E7B6) via USB MIDI")
        XCTAssertNil(set.errorMessage, "the error follows the member in use")
        set.sendControlRequest(request: 0x54, value: 0, index: 2, data: Data([0, 0, 0, 0]))
        XCTAssertEqual(midi.sets, [0x54])
        XCTAssertEqual(usb.sets, [])

        // The vendor path comes back (replugged): it takes over again.
        usb.openByItself(usbDevice)
        XCTAssertEqual(midi.disconnects, 1, "MIDI lets go of the port")
        XCTAssertEqual(set.selectedDevice?.remoteName, nil)
        set.sendControlRequest(request: 0x54, value: 0, index: 2, data: Data([0, 0, 0, 0]))
        XCTAssertEqual(usb.sets, [0x54])
    }

    func testSWF2NoFallbackWithoutTheSameDeviceOnMIDI() {
        let usb = ScriptedMember([usbDevice], opens: false)
        let midi = ScriptedMember([], opens: true)
        let set = LocalTransportSet([usb, midi])
        usb.failOpen()
        XCTAssertEqual(midi.selectCalls, [])
        XCTAssertFalse(set.isConnected)
        XCTAssertEqual(set.errorMessage, "Could not open device", "a clean error, not a stuck badge")
    }

    func testSWF2AnErrorWithADeviceSelectedIsNotAFailedOpen() {
        let usb = ScriptedMember([usbDevice], opens: true)
        let midi = ScriptedMember([midiDevice], opens: true)
        let set = LocalTransportSet([usb, midi])
        usb.openByItself(usbDevice)
        // USBDevice keeps the selection on removal, for the auto-reconnect.
        usb.connected.send(false)
        usb.error.send("Device Removed")
        XCTAssertEqual(midi.selectCalls, [])
        XCTAssertEqual(set.errorMessage, "Device Removed")
    }

    func testSWF2TheLockGoesToTheMemberInUse() {
        let usb = ScriptedMember([usbDevice], opens: true)
        let midi = ScriptedMember([midiDevice], opens: true)
        let set = LocalTransportSet([usb, midi])
        XCTAssertFalse(set.acquireDeviceLock(reason: "test", timeout: 1), "the member's answer, not the default")
        XCTAssertEqual(usb.lockCalls, 1)
        XCTAssertEqual(midi.lockCalls, 0)
    }

    // -- WP SW-M1 -- the held-device case moved with USB to the core:
    // CoreUsbSWM1Tests.testSWM1AHeldDeviceOpensAfterTheRetriesOrSaysBusy.
}

// MARK: - F3: a refused write is put back on screen

final class SWF2RefusedWriteTests: XCTestCase {

    private func waitForNotice(_ message: String) -> Bool {
        spinUntil(3) { DeviceLock.notice.message == message }
    }

    func testSWF2ADeniedWriteAsksForAReReadAndSaysWhy() {
        let fanout = NotificationFanout()
        var packets: [Data] = []
        let sub = fanout.add { packets.append($0.packet) }
        XCTAssertTrue(RefusedWrite.handle(.denied, fanout: fanout))
        XCTAssertTrue(waitForNotice(DeviceLock.deniedMessage))   // -- WP P1SW3 -- the core's `denied` row
        XCTAssertEqual(packets, [Data([0x02, 0x03, 0, 0, 0, 0, 0, 0])], "a BULK_INVALIDATED: re-read everything")
        XCTAssertTrue(RefusedWrite.handle(.locked, fanout: fanout))
        XCTAssertTrue(waitForNotice(DeviceLock.lockedMessage))
        XCTAssertEqual(packets.count, 2)
        for status: LinkStatus in [.ok, .error, .timeout, .noDevice, .rateLimited] {
            XCTAssertFalse(RefusedWrite.handle(status, fanout: fanout), "\(status) is not a refusal")
        }
        XCTAssertEqual(packets.count, 2)
        sub.cancel()
    }

    /// The hw-mac case end to end: a viewer's edit on a hub-shared device.
    /// The hub (the Link client fake) answers SETs DENIED and GETs from the
    /// demo device, which therefore never changes.
    func testSWF2AViewersEditRevertsAndThePresetIsNotDirty() throws {
        let demo = FakeTransport(autoSelect: false)
        demo.selectDevice(FakeTransport.demoDevice)
        let client = LinkClientFake()
        client.responder = { req in
            if req.direction == .set { return LinkCmdResponse(tag: req.tag, status: .denied) }
            switch demo.getControlResult(request: req.bRequest, value: req.wValue, index: req.wIndex,
                                         length: req.wLength) {
            case .success(let data): return LinkCmdResponse(tag: req.tag, status: .ok, payload: data)
            case .failure(let status): return LinkCmdResponse(tag: req.tag, status: status)
            }
        }
        let remote = NetworkTransport(client: client, hubID: "hub-swf2", hubName: "Linux hub",
                                      tokens: LinkTokenStore(service: "test-\(UUID().uuidString)"),
                                      clientName: "Tester")
        let vm = DSPViewModel(transport: remote)
        // The app wires BULK_INVALIDATED to the resync through its shared
        // monitor; this one watches the remote transport the same way.
        let monitor = InterruptMonitor(transport: remote)
        monitor.onBulkInvalidated = { [weak vm] _ in vm?.scheduleDeviceResync() }
        monitor.start()

        remote.bind(serial: FakeTransport.demoDevice.serial)
        client.setDevices([LinkDeviceInfo(handle: 1, serial: FakeTransport.demoDevice.serial,
                                          name: "DSPi", state: .online)])
        client.setState(.ready)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.isDeviceReady }, "connected through the hub")
        // Writes the connect path makes are refused too (hw-mac O4); let their
        // notice and re-read pass, so what follows is this edit's own.
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
        let original = vm.outputGainDB[0]
        vm.updateSavedSnapshot()
        XCTAssertFalse(vm.hasUnsavedChanges)

        vm.setOutputGain(output: 0, db: original - 3)
        XCTAssertEqual(vm.outputGainDB[0], original - 3, "applied locally first, as before")
        XCTAssertTrue(vm.hasUnsavedChanges)
        XCTAssertEqual(client.commands.last?.direction, .set)

        XCTAssertTrue(spinUntil(10) { vm.outputGainDB[0] == original }, "the device's value is back")
        XCTAssertFalse(vm.hasUnsavedChanges, "no dirty preset for a write that never happened")
        XCTAssertTrue(waitForNotice(DeviceLock.deniedMessage))   // -- WP P1SW3 -- the core's `denied` row
        monitor.stop()
    }
}

// MARK: - F4: the voltage rule at the Swift hub

final class SWF2CoreVoltageRuleTests: XCTestCase {

    private func status(live: UInt8, stored: UInt8, defaultVreg: UInt8, trial: Bool = false) -> CoreVregStatus {
        var b = [UInt8](repeating: 0, count: CoreVregStatus.size)
        b[0] = live; b[1] = stored; b[2] = defaultVreg; b[5] = trial ? 1 : 0
        return CoreVregStatus(Data(b))!
    }

    /// The core's own cases (overvolt.rs `wp_cg14_the_owners_cases`).
    func testSWF2TheOwnersCases() {
        for (current, target, needs) in [(1250, 1300, true), (1150, 1200, false), (1200, 1250, true),
                                         (1150, 1250, true), (1250, 1250, false), (1300, 1250, false),
                                         (1300, 1300, false), (1200, 1200, false), (1200, 1150, false)] {
            XCTAssertEqual(CoreVoltageRule.wordRequired(currentMV: UInt16(current), targetMV: UInt16(target)), needs,
                           "\(current) -> \(target)")
        }
        XCTAssertTrue(CoreVoltageRule.wordRequired(currentMV: nil, targetMV: 1250))
        XCTAssertFalse(CoreVoltageRule.wordRequired(currentMV: nil, targetMV: 1200))
    }

    /// The Rust hub's table (router.rs `wp_hb2_the_voltage_rule_at_the_hub`).
    func testSWF2TheHubsTable() {
        let k340 = status(live: 0x0D, stored: 0, defaultVreg: 0x0D)
        XCTAssertTrue(CoreVoltageRule.needsWord(.set(0x0F), status: k340))
        XCTAssertTrue(CoreVoltageRule.needsWord(.trial(0x0F), status: k340))
        XCTAssertTrue(CoreVoltageRule.needsWord(.set(0x0E), status: k340))
        XCTAssertTrue(CoreVoltageRule.needsWord(.trial(0x0E), status: k340))
        XCTAssertFalse(CoreVoltageRule.needsWord(.set(0x0D), status: k340), "level")
        XCTAssertFalse(CoreVoltageRule.needsWord(.set(0x0C), status: k340), "lower")
        XCTAssertFalse(CoreVoltageRule.needsWord(.set(0), status: k340), "the build default")
        XCTAssertTrue(CoreVoltageRule.needsWord(.trial(0x40), status: k340), "not a voltage")
        let running125 = status(live: 0x0E, stored: 0x0E, defaultVreg: 0x0D)
        XCTAssertFalse(CoreVoltageRule.needsWord(.set(0x0E), status: running125), "already there")
        // A trial at 1.25 V does not make a SET to 1.25 V level: it persists it.
        let trial125 = status(live: 0x0E, stored: 0, defaultVreg: 0x0D, trial: true)
        XCTAssertTrue(CoreVoltageRule.needsWord(.set(0x0E), status: trial125))
        XCTAssertFalse(CoreVoltageRule.needsWord(.trial(0x0E), status: trial125))
        for op: CoreVregOp in [.trialEnd, .auto(true), .clear] {
            XCTAssertFalse(CoreVoltageRule.needsWord(op, status: k340))
        }
    }

    /// Router.rs `wp_hb2_an_unreadable_voltage_counts_as_needing_the_word`.
    func testSWF2AnUnreadableUnitNeedsTheWord() {
        XCTAssertTrue(CoreVoltageRule.needsWord(.set(0x0E), status: nil))
        XCTAssertTrue(CoreVoltageRule.needsWord(.trial(0x0F), status: nil))
        XCTAssertTrue(CoreVoltageRule.needsWord(.set(0), status: nil))
        XCTAssertFalse(CoreVoltageRule.needsWord(.set(0x0D), status: nil))
        XCTAssertFalse(CoreVoltageRule.needsWord(.trial(0x0C), status: nil))
        XCTAssertFalse(CoreVoltageRule.needsWord(.trialEnd, status: nil))
    }

    func testSWF2PayloadsDecodeAsTheFirmwareDefinesThem() {
        XCTAssertEqual(CoreVregOp(payload: Data([0, 0x0E])), .set(0x0E))
        XCTAssertEqual(CoreVregOp(payload: Data([1, 0x0E])), .trial(0x0E))
        XCTAssertEqual(CoreVregOp(payload: Data([2, 9])), .trialEnd)
        XCTAssertEqual(CoreVregOp(payload: Data([3, 1])), .auto(true))
        XCTAssertEqual(CoreVregOp(payload: Data([4, 0])), .clear)
        XCTAssertNil(CoreVregOp(payload: Data([3, 2])))
        XCTAssertNil(CoreVregOp(payload: Data([5, 0])))
        XCTAssertNil(CoreVregOp(payload: Data([0])))
        XCTAssertNil(CoreVregOp(payload: Data([0, 0x0E, 0])))
    }
}

#if os(macOS)
final class SWF2HubVoltageTests: XCTestCase {

    /// A K340 at 1.20 V: answers EXT_CORE_VREG reads (or not) and takes SETs.
    private func k340(readable: Bool = true) -> LinkFakeDevice {
        let device = LinkFakeDevice()
        device.responder = { req in
            if req.direction == .get, req.bRequest == REQ_EXTENSION, req.wValue == EXT_CORE_VREG {
                guard readable else { return LinkCmdResponse(tag: req.tag, status: .error) }
                var b = [UInt8](repeating: 0, count: CoreVregStatus.size)
                b[0] = 0x0D; b[2] = 0x0D
                return LinkCmdResponse(tag: req.tag, status: .ok, payload: Data(b))
            }
            return LinkCmdResponse(tag: req.tag, status: .ok)
        }
        return device
    }

    private func vreg(_ op: UInt8, _ value: UInt8) -> LinkCmdRequest {
        LinkCmdRequest(tag: 9, handle: 0, direction: .set, bRequest: REQ_EXTENSION,
                       wValue: EXT_CORE_VREG, wIndex: EXT_WINDEX, payload: Data([op, value]))
    }

    private func run(_ request: LinkCmdRequest, on router: CommandRouter, as session: RouterSession) -> LinkStatus {
        let done = expectation(description: "routed")
        var status: LinkStatus = .ok
        router.submit(request, session: session) { result in status = result.response.status; done.fulfill() }
        wait(for: [done], timeout: 5)
        return status
    }

    private func vregSets(_ device: LinkFakeDevice) -> [Data] {
        device.executed.filter { $0.direction == .set && $0.wValue == EXT_CORE_VREG }.map { $0.payload }
    }

    func testSWF2ANetworkAdminCannotRaiseTheVoltage() {
        let device = k340()
        let router = CommandRouter(handle: 0, device: device, policy: LinkPolicy.bundled ?? .empty)
        let admin = RouterSession(id: 2, role: .admin)
        XCTAssertEqual(run(vreg(0, 0x0E), on: router, as: admin), .denied, "SET 1.25 V")
        XCTAssertEqual(run(vreg(1, 0x0E), on: router, as: admin), .denied, "TRIAL 1.25 V")
        XCTAssertEqual(run(vreg(0, 0x0F), on: router, as: admin), .denied, "SET 1.30 V")
        XCTAssertEqual(run(vreg(9, 0x0E), on: router, as: admin), .denied, "not a defined op: refused unread")
        XCTAssertEqual(vregSets(device), [], "no raise reached the device")
        // Lowering, level, the build default, and the non-raising ops pass.
        for (op, value) in [(0, 0x0D), (0, 0x0C), (0, 0), (2, 0), (3, 1), (4, 0)] as [(UInt8, UInt8)] {
            XCTAssertEqual(run(vreg(op, value), on: router, as: admin), .ok, "op \(op) \(value)")
        }
        XCTAssertEqual(vregSets(device).count, 6)
    }

    func testSWF2TheAppsSessionIsExempt() {
        let device = k340()
        let router = CommandRouter(handle: 0, device: device, policy: LinkPolicy.bundled ?? .empty)
        let app = RouterSession(id: LinkHub.localSessionID, role: .admin, exemptFromInflightCap: true)
        XCTAssertEqual(run(vreg(0, 0x0E), on: router, as: app), .ok, "the app asks for the word itself")
        XCTAssertEqual(vregSets(device), [Data([0, 0x0E])])
        XCTAssertFalse(device.executed.contains { $0.direction == .get }, "and the hub reads nothing for it")
    }

    func testSWF2AnUnreadableUnitIsRefusedAboveTheLimit() {
        let device = k340(readable: false)
        let router = CommandRouter(handle: 0, device: device, policy: LinkPolicy.bundled ?? .empty)
        let admin = RouterSession(id: 3, role: .admin)
        XCTAssertEqual(run(vreg(0, 0x0E), on: router, as: admin), .denied)
        XCTAssertEqual(run(vreg(0, 0), on: router, as: admin), .denied, "the build default is unknown")
        XCTAssertEqual(run(vreg(0, 0x0D), on: router, as: admin), .ok, "1.20 V never needs the word")
        XCTAssertEqual(vregSets(device), [Data([0, 0x0D])])
    }

    func testSWF2RolesStillComeFirst() {
        let device = k340()
        let router = CommandRouter(handle: 0, device: device, policy: LinkPolicy.bundled ?? .empty)
        XCTAssertEqual(run(vreg(0, 0x0C), on: router, as: RouterSession(id: 4, role: .control)), .denied,
                       "0x00 is config: admin only")
        XCTAssertTrue(device.executed.isEmpty, "refused before the device was touched")
    }
}
#endif
