//
//  CoreHubSWM3Tests.swift
//  DSPi ConsoleTests
//
//  WP SW-M3.  The Mac's DSPi Link hub from the Rust core (Core/CoreHub.swift,
//  `CoreHubService` over `dspi_hub.HubHost`), in process on loopback, with the
//  core's own Link session as the client.  No USB: the hub serves its demo
//  device, and the app's core reaches it through the hub (`Link.usb` with
//  `direct: false`, so only a hub's devices show).
//
//  - The service: sharing on and off, the advertisement, pairing with the
//    window's PIN, roles, a revoke, a rename and a change of access mode that
//    forgets paired clients only after a yes.
//  - The app's core on the hub (`HubHost.attachApp`): the local transport
//    lists and opens the hub's device through the hub while sharing, and the
//    hub's stop gives the core its own path back.
//  - Opt-in (`TEST_RUNNER_DSPI_SWM3_CONF=<dir>`): the app's hub serving HB7's
//    Swift conformance client (crates/dspi-hub/tests/conformance/swift), its
//    `@ctl` commands through files as serve.rs takes them, driven by
//    apps/apple/Tools/hb7-app-conformance.sh.
//

#if os(macOS)
import XCTest
import dspi_core
import dspi_hub
@testable import DSPi_Console

/// Spin the main run loop until `condition` holds.
@discardableResult
private func swm3Wait(_ timeout: TimeInterval, _ what: String, file: StaticString = #filePath, line: UInt = #line,
                      _ condition: () -> Bool) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while !condition() {
        if Date() > deadline {
            XCTFail("timed out waiting for \(what)", file: file, line: line)
            return false
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    }
    return true
}

/// Run `body` off main (the core's Link calls block) and spin until it is done.
private func swm3OffMain<T>(_ what: String, timeout: TimeInterval = 20, _ body: @escaping () throws -> T) throws -> T {
    var result: Result<T, Error>?
    let lock = NSLock()
    DispatchQueue.global().async {
        let r = Result { try body() }
        lock.lock(); result = r; lock.unlock()
    }
    swm3Wait(timeout, what) { lock.lock(); defer { lock.unlock() }; return result != nil }
    lock.lock(); defer { lock.unlock() }
    guard let r = result else { throw NSError(domain: "swm3", code: 1) }
    return try r.get()
}

/// What the service advertised.
final class SWM3Advertiser: HubAdvertising {
    var started: (name: String, port: Int, ad: LinkAdvertisement)?
    var updates: [LinkAdvertisement] = []
    var stopped = 0
    func start(name: String, port: Int, ad: LinkAdvertisement) { started = (name, port, ad) }
    func updateTXT(_ ad: LinkAdvertisement) { updates.append(ad) }
    func stop() { stopped += 1 }
}

/// A scratch store and preferences for one service.
private func swm3Setup(_ name: String, usb: Bool = false) -> CoreHubService.Setup {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("swm3-\(name)-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
    let suite = "swm3.\(name).\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: suite)!
    defaults.set(0, forKey: "LinkSharingPort")   // any free port
    var setup = CoreHubService.Setup()
    setup.storeDir = dir
    setup.loopback = true
    setup.web = false
    setup.usb = usb
    setup.defaults = defaults
    return setup
}

@MainActor
private func swm3URL(_ service: CoreHubService) -> String {
    "ws://127.0.0.1:\(service.boundPort ?? 0)/dspi/v1"
}

@MainActor
final class CoreHubSWM3Tests: XCTestCase {

    /// Pair a core client with the service's window PIN; returns the core.
    private func pairClient(_ service: CoreHubService, name: String) throws -> DspiCore {
        service.allowNewClient()
        swm3Wait(5, "a PIN") { service.activePIN != nil }
        let pin = try XCTUnwrap(service.activePIN)
        let url = swm3URL(service)
        let client = DspiCore()
        let hello = try swm3OffMain("connect") {
            try client.linkConnect(url: url, clientName: name, appVersion: "0", token: "", expectedHubId: "")
        }
        XCTAssertTrue(hello.needsPairing)
        XCTAssertEqual(hello.hubId, service.hubID)
        let token = try swm3OffMain("pair") { try client.linkPair(pin: pin) }
        XCTAssertFalse(token.isEmpty)
        return client
    }

    func testSWM3SharingPairingRolesRevoke() throws {
        let ad = SWM3Advertiser()
        let service = CoreHubService(devices: .demo, setup: swm3Setup("pair"), discovery: ad)
        XCTAssertFalse(service.isRunning)
        XCTAssertFalse(service.sharingEnabled)
        service.setSharing(true)
        swm3Wait(10, "the hub running") { service.isRunning }
        XCTAssertNil(service.lastError)
        XCTAssertTrue(service.sharingEnabled)
        let port = try XCTUnwrap(service.boundPort)
        XCTAssertGreaterThan(port, 0)
        let hubID = try XCTUnwrap(service.hubID)
        XCTAssertEqual(ad.started?.port, port)
        XCTAssertEqual(ad.started?.name, service.hubName)
        XCTAssertEqual(ad.started?.ad.hubID, hubID)
        XCTAssertEqual(ad.started?.ad.auth, "pin")
        // Keep-awake while hosting (App Nap off; idle sleep only when asked).
        XCTAssertTrue(service.holdsPowerActivityForTest)
        // The demo device is advertised once the hub lists it.
        swm3Wait(5, "the demo device in the TXT") { ad.updates.last?.deviceCount == 1 }
        XCTAssertEqual(ad.updates.last?.serials, ["DEMO0000DSPI0001"])
        XCTAssertNotNil(service.listenAddress.map { $0.hasSuffix(":\(port)") } ?? true)

        // Pairing: the window's PIN, then the client and its session show.
        let client = try pairClient(service, name: "swm3 client")
        swm3Wait(5, "the paired client") { service.clientRows.count == 1 }
        swm3Wait(5, "the session") { service.sessionCount == 1 }
        swm3Wait(5, "the window closed by the right PIN") { service.activePIN == nil }
        let row = try XCTUnwrap(service.clientRows.first)
        XCTAssertEqual(row.name, "swm3 client")
        XCTAssertEqual(row.role, .control, "a PIN gives control (the hub's rule)")

        // Roles are the hub's.
        service.setRole(row, .admin)
        swm3Wait(5, "the admin role") { service.clientRows.first?.role == .admin }

        // A revoke ends the client's session and forgets it.
        service.revoke(row)
        swm3Wait(5, "the client forgotten") { service.clientRows.isEmpty }
        swm3Wait(5, "its session closed") { service.sessionCount == 0 }
        client.linkClose()

        // A rename restarts the hub under the new name, same id.
        service.hubName = "SWM3 renamed"
        swm3Wait(10, "the restart") { service.isRunning && ad.started?.name == "SWM3 renamed" }
        XCTAssertEqual(service.statusForTest().name, "SWM3 renamed")
        XCTAssertEqual(service.hubID, hubID)

        // Open access forgets every paired client: asked first.
        let second = try pairClient(service, name: "swm3 second")
        swm3Wait(5, "the second client") { service.clientRows.count == 1 }
        service.authMode = .none
        XCTAssertEqual(service.authMode, .pin, "nothing changes before the yes")
        XCTAssertEqual(service.accessChangeCost, 1)
        service.cancelAccessChange()
        XCTAssertNil(service.accessChangeCost)
        XCTAssertEqual(service.statusForTest().access, .pin)
        service.authMode = .none
        service.confirmAccessChange()
        XCTAssertEqual(service.authMode, .none)
        swm3Wait(10, "the restart in open access") { service.isRunning && service.statusForTest().access == .open }
        XCTAssertEqual(service.statusForTest().tokensRevoked, 1)
        XCTAssertEqual(ad.started?.ad.auth, "none")
        second.linkClose()

        service.setSharing(false)
        XCTAssertFalse(service.isRunning)
        XCTAssertFalse(service.sharingEnabled)
        swm3Wait(10, "the hub stopped") { !service.statusForTest().running }
        XCTAssertGreaterThanOrEqual(ad.stopped, 1)
        XCTAssertFalse(service.holdsPowerActivityForTest)
    }

    /// A port in use: the start fails, says why, and sharing stays off.
    func testSWM3PortInUseIsReported() throws {
        let first = CoreHubService(devices: .demo, setup: swm3Setup("busy1"), discovery: SWM3Advertiser())
        first.setSharing(true)
        swm3Wait(10, "the first hub") { first.isRunning }
        let setup = swm3Setup("busy2")
        setup.defaults.set(first.boundPort ?? 0, forKey: "LinkSharingPort")
        let second = CoreHubService(devices: .demo, setup: setup, discovery: SWM3Advertiser())
        second.setSharing(true)
        swm3Wait(10, "the failure") { second.lastError != nil }
        XCTAssertFalse(second.isRunning)
        XCTAssertFalse(second.sharingEnabled)
        XCTAssertTrue(second.lastError?.contains("Could not start on port") == true, second.lastError ?? "")
        first.setSharing(false)
    }

    /// The app's core on the hub: while sharing, the local transport gets the
    /// hub's device through the hub and opens it; the stop gives it back.
    func testSWM3AppCoreThroughTheHub() throws {
        let transport = CoreBackedTransport(link: .usb(direct: false))
        XCTAssertTrue(transport.isUsbLink)
        let service = CoreHubService(devices: .app(transport), setup: swm3Setup("app"), discovery: SWM3Advertiser())
        // No direct USB here: nothing listed before sharing.
        RunLoop.main.run(until: Date().addingTimeInterval(0.8))
        XCTAssertTrue(transport.availableDevices.isEmpty)
        XCTAssertFalse(transport.isConnected)

        service.setSharing(true)
        swm3Wait(10, "the hub running") { service.isRunning }
        swm3Wait(5, "the core on the hub") { transport.usbBrokeredForTest }
        // The hub's device (its demo here; the bus's DSPis in the app).
        let host = service.host
        let handle = try swm3OffMain("attach the demo") { try host.attachDemo() }
        XCTAssertEqual(handle, 0)
        swm3Wait(10, "the hub's device listed and opened") {
            transport.availableDevices.map(\.serial) == ["DEMO0000DSPI0001"] && transport.isConnected
        }
        XCTAssertEqual(transport.selectedDevice?.serial, "DEMO0000DSPI0001")
        // The core reads the device through the hub.
        XCTAssertNotNil(transport.getControlRequest(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 6))

        // A network client sees the same device, and the app's write.
        let client = try pairClient(service, name: "swm3 viewer")
        swm3Wait(5, "the client") { service.clientRows.count == 1 }
        let serials = try swm3OffMain("the client's device list") { () -> [String] in
            for _ in 0..<100 where client.linkDevices().isEmpty { Thread.sleep(forTimeInterval: 0.05) }
            return client.linkDevices().map(\.serial)
        }
        XCTAssertEqual(serials, ["DEMO0000DSPI0001"])
        client.linkClose()

        // The hub's stop: the core's own path again (no direct USB here: no device).
        service.setSharing(false)
        swm3Wait(10, "the core off the hub") { !transport.usbBrokeredForTest }
        swm3Wait(10, "the hub's device gone from the local list") {
            transport.availableDevices.isEmpty && !transport.isConnected
        }
    }

    // MARK: - HB7's Swift client against the app's hub (opt-in)

    /// Serve HB7's conformance driver (serve.rs's protocol: `hub.json`, `cmd`,
    /// `ack` in the directory) from the app's own hub service, until `stop`.
    func testSWM3ServeHB7SwiftClient() throws {
        let env = ProcessInfo.processInfo.environment
        guard let dirPath = env["DSPI_SWM3_CONF"], !dirPath.isEmpty else {
            throw XCTSkip("Opt-in: TEST_RUNNER_DSPI_SWM3_CONF=<dir> (apps/apple/Tools/hb7-app-conformance.sh).")
        }
        let dir = URL(fileURLWithPath: dirPath, isDirectory: true)
        let fm = FileManager.default
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try? fm.removeItem(at: dir.appendingPathComponent("cmd"))
        try? fm.removeItem(at: dir.appendingPathComponent("ack"))
        var setup = swm3Setup("hb7")
        setup.defaults.set(Int(env["DSPI_SWM3_PORT"] ?? "") ?? 0, forKey: "LinkSharingPort")
        setup.web = true
        // The Local Network check: every interface, as the app's hub listens.
        if env["DSPI_SWM3_LAN"] == "1" { setup.loopback = false }
        let service = CoreHubService(devices: .demo, setup: setup, discovery: SWM3Advertiser())
        service.hubName = "HB7 on the Mac app"
        service.setSharing(true)
        swm3Wait(10, "the hub running") { service.isRunning }
        XCTAssertNil(service.lastError)
        swm3Wait(5, "the demo device") { service.sharedSerials.count == 1 }

        func write(_ name: String, _ text: String) throws {
            let tmp = dir.appendingPathComponent(".\(name).tmp")
            try text.write(to: tmp, atomically: false, encoding: .utf8)
            guard rename(tmp.path, dir.appendingPathComponent(name).path) == 0 else {
                throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
            }
        }
        func json(_ object: [String: Any]) -> String {
            String(data: try! JSONSerialization.data(withJSONObject: object), encoding: .utf8)!
        }
        func newPin() -> String {
            let old = service.activePIN
            if old != nil { service.cancelPairing() }
            service.allowNewClient()
            swm3Wait(5, "a new PIN") { service.activePIN != nil && service.activePIN != old }
            return service.activePIN ?? ""
        }
        func row(_ name: String) -> HubClientRow? {
            service.refreshClients()
            swm3Wait(5, "the client \(name)") { service.clientRows.contains { $0.name == name } }
            return service.clientRows.first { $0.name == name }
        }

        let hello: [String: Any] = ["port": service.boundPort ?? 0, "hub_id": service.hubID ?? "", "pin": newPin()]
        try write("hub.json", json(hello))
        print("SWM3 serving \(service.hubID ?? "?") on port \(service.boundPort ?? 0)")

        let secs = TimeInterval(env["DSPI_SWM3_SECS"] ?? "") ?? 600
        let end = Date().addingTimeInterval(secs)
        var stopped = false
        while Date() < end && !stopped {
            guard let text = try? String(contentsOf: dir.appendingPathComponent("cmd"), encoding: .utf8) else {
                RunLoop.main.run(until: Date().addingTimeInterval(0.02))
                continue
            }
            try? fm.removeItem(at: dir.appendingPathComponent("cmd"))
            let words = text.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: " ", maxSplits: 2).map(String.init)
            let n = words.first ?? "0", verb = words.count > 1 ? words[1] : "", arg = words.count > 2 ? words[2] : ""
            print("SWM3 command \(n): \(verb) \(arg)")
            var answer: [String: Any] = [:]
            switch verb {
            case "pin":
                answer = ["pin": newPin()]
            case "admin":
                if let r = row(arg) {
                    service.setRole(r, .admin)
                    answer = ["promoted": swm3Wait(5, "\(arg) made admin") { service.clientRows.first { $0.id == r.id }?.role == .admin }]
                } else {
                    answer = ["promoted": false]
                }
            case "change":
                answer = ["changed": service.host.demoTurnKnob()]
            case "revoke":
                if let r = row(arg) {
                    service.revoke(r)
                    answer = ["revoked": swm3Wait(5, "\(arg) revoked") { !service.clientRows.contains { $0.id == r.id } }]
                } else {
                    answer = ["revoked": false]
                }
            case "stop":
                service.setSharing(false)
                swm3Wait(10, "the hub stopped") { !service.statusForTest().running }
                stopped = true
            default:
                answer = ["error": "unknown command \(verb)"]
            }
            try write("ack", "\(n) \(json(answer))")
        }
        XCTAssertTrue(stopped, "no stop from the driver in \(secs) s")
        // Long enough for the driver to read the last answer.
        RunLoop.main.run(until: Date().addingTimeInterval(0.3))
        if !stopped { service.setSharing(false) }
    }
}
#endif
