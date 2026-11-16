//
//  HubP2SWTests.swift
//  DSPi ConsoleTests
//
//  WP P2SW.  The hub's families on the Swift side, and the Mac hub's
//  security and services.
//
//  - Both platforms: the alarm rules and notices (HubServerModel), the
//    notice host's Acknowledge, the merged snapshot list's sections
//    (SnapshotsHubModel), the events' lines.
//  - macOS: the Mac hub (CoreHubService over dspi_hub): a hub with default
//    options requires TLS, shows its fingerprint, advertises tls=1 and runs
//    no optional service; the compatibility mode asks first and serves
//    both; the services run when asked, and a core client on the Mac hub
//    sees them (caps, Identify, the history, an alarm list).
//

import XCTest
import dspi_core
#if os(macOS)
import dspi_hub
#endif
@testable import DSPi_Console

private func p2swAlarm(_ id: String, serial: String = "", state: String = "raised",
                       code: String = "device_restarted") -> HubAlarm {
    HubAlarm(id: id, source: "device", code: code, subject: serial, serial: serial, severity: "warning",
             message: "Restarted", state: state, raisedAt: "2026-11-16T10:00:00Z", clearedAt: "",
             acknowledgedBy: "", count: 1, reason: "watchdog", noticeKey: "alarm.\(code)")
}

private func p2swSnapshot(_ id: String, _ location: String, localCopy: Bool = false) -> MergedSnapshot {
    MergedSnapshot(snapshot: SnapshotInfo(id: id, name: "S \(id)", note: "", serial: "DEMO0000DSPI0001",
                                          savedMs: 1, platform: "RP2350", inputChannels: 2, outputChannels: 4,
                                          wireFormat: 1),
                   location: location, localCopy: localCopy)
}

final class HubP2SWModelTests: XCTestCase {

    func testP2SWAlarmsOfTheOpenDeviceAndTheHub() {
        let all = [p2swAlarm("a", serial: "ONE"), p2swAlarm("b", serial: "TWO"), p2swAlarm("c"),
                   p2swAlarm("d", serial: "ONE", state: "cleared")]
        XCTAssertEqual(HubServerModel.relevant(all, serial: "ONE").map(\.id), ["a", "c"])
        // A change: a new one first, a cleared one leaves, another device's ignored.
        var now = HubServerModel.relevant(all, serial: "ONE")
        now = HubServerModel.applyChange(now, p2swAlarm("e", serial: "ONE"), serial: "ONE")
        XCTAssertEqual(now.map(\.id), ["e", "a", "c"])
        now = HubServerModel.applyChange(now, p2swAlarm("a", serial: "ONE", state: "cleared"), serial: "ONE")
        XCTAssertEqual(now.map(\.id), ["e", "c"])
        now = HubServerModel.applyChange(now, p2swAlarm("f", serial: "TWO"), serial: "ONE")
        XCTAssertEqual(now.map(\.id), ["e", "c"])
    }

    func testP2SWAlarmNoticeIsTheCoresRowWithAcknowledge() {
        let alarm = p2swAlarm("x1", serial: "ONE")
        let notice = HubServerModel.notice(for: alarm)
        let row = hubAlarmNotice(alarm: alarm, time: "")
        XCTAssertEqual(notice.title, row.title)
        XCTAssertEqual(notice.reason, row.reason)
        XCTAssertFalse(notice.title.isEmpty)
        XCTAssertEqual(notice.alarmID, "x1")

        // The host's Acknowledge calls the hub with the alarm's id, then dismisses.
        let center = NoticeCenter(infoSeconds: 60)
        var acked: [String] = []
        center.acknowledge = { acked.append($0) }
        center.post(notice)
        XCTAssertEqual(center.current?.alarmID, "x1")
        center.acknowledge?(center.current?.alarmID ?? "")
        XCTAssertEqual(acked, ["x1"])
        // A plain notice offers none.
        XCTAssertEqual(ConsoleNotice.result("done").alarmID, "")
    }

    func testP2SWMergedSnapshotSections() {
        let list = [p2swSnapshot("1", "hub"), p2swSnapshot("2", "both"), p2swSnapshot("3", "local"),
                    p2swSnapshot("2", "local", localCopy: true)]
        XCTAssertEqual(SnapshotsHubModel.onHub(list).map(\.snapshot.id), ["1", "2"])
        XCTAssertEqual(SnapshotsHubModel.localOnly(list).map(\.snapshot.id), ["3"],
                       "a local copy of a differing id is not offered for Save to the hub")
        XCTAssertEqual(SnapshotsHubModel.whereLine(list[0]), "On the hub only")
        XCTAssertEqual(SnapshotsHubModel.whereLine(list[1]), "Also on this console")
    }

    func testP2SWEventLines() {
        let r = HubEventRecord(id: 7, timeMs: 1_700_000_000_000, kind: "param", serial: "ONE", text: "Volume -6 dB",
                               source: "remote", session: 3, by: "Kitchen iPad", channel: nil, value: nil, fw: nil,
                               uptimeS: nil, lines: [], count: 1)
        let line = EventLine(r)
        XCTAssertEqual(line.id, "h7")
        XCTAssertEqual(line.by, "Kitchen iPad")
        XCTAssertEqual(line.text, "Volume -6 dB")
        XCTAssertFalse(line.time.isEmpty)
        let e = EventEntry(id: 7, timeMs: 1, kind: "param", serial: "ONE", text: "Mute on", source: "local",
                           session: nil, own: true, channel: nil, value: nil, fw: nil, uptimeS: nil, lines: [],
                           count: 1)
        XCTAssertEqual(EventLine(e).id, "l7", "the two logs' ids never collide")
        XCTAssertEqual(EventLine(e).by, "")
    }
}

#if os(macOS)

/// Spin the main run loop until `condition` holds.
@MainActor
private func p2swWait(_ seconds: TimeInterval, _ what: String, _ condition: () -> Bool) {
    let end = Date().addingTimeInterval(seconds)
    while !condition() && Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
    XCTAssertTrue(condition(), "timed out waiting for \(what)")
}

/// `body` off main, waiting on main's run loop.
@MainActor
private func p2swOffMain<T>(_ what: String, _ body: @escaping () throws -> T) throws -> T {
    var result: Result<T, Error>?
    DispatchQueue.global().async {
        let r = Result { try body() }
        DispatchQueue.main.async { result = r }
    }
    p2swWait(20, what) { result != nil }
    return try XCTUnwrap(result).get()
}

private final class P2SWAdvertiser: HubAdvertising {
    var started: (name: String, port: Int, ad: LinkAdvertisement)?
    var updates: [LinkAdvertisement] = []
    func start(name: String, port: Int, ad: LinkAdvertisement) { started = (name, port, ad) }
    func updateTXT(_ ad: LinkAdvertisement) { updates.append(ad) }
    func stop() {}
}

/// A scratch store and preferences; loopback, any free port, the app's TLS
/// unless `tls` says otherwise.
private func p2swSetup(_ name: String, tls: HubTls? = nil) -> CoreHubService.Setup {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("p2sw-\(name)-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
    let defaults = UserDefaults(suiteName: "p2sw.\(name).\(UUID().uuidString)")!
    defaults.set(0, forKey: "LinkSharingPort")
    var setup = CoreHubService.Setup()
    setup.storeDir = dir
    setup.loopback = true
    setup.web = false
    setup.usb = false
    setup.defaults = defaults
    setup.tls = tls
    return setup
}

/// Every cap an optional service adds to `hello`.
private let p2swOptionalCaps = ["health", "inventory", "identify", "events", "alarms", "groups", "schedule"]

@MainActor
final class HubP2SWMacHubTests: XCTestCase {

    func testP2SWDefaultOptions() {
        let o = hubDefaultStartOptions(storeDir: "/x")
        XCTAssertEqual(o.tls, .require)
        XCTAssertFalse(o.upstreamCompat)
        XCTAssertEqual(o.services, HubServices(health: false, inventory: false, identify: false, history: false,
                                               alarms: false, groups: false))
        // The app's own start options from its defaults (a fresh preferences suite).
        var setup = CoreHubService.Setup()
        setup.defaults = UserDefaults(suiteName: "p2sw.defaults.\(UUID().uuidString)")!
        let services = CoreHubService.load(setup.defaults)
        let app = CoreHubService.startOptions(name: "Mac", port: 11915, access: .pin, usb: true, app: true,
                                              compat: false, services: services, setup: setup, mdnsName: nil)
        XCTAssertEqual(app.tls, .require, "TLS required by default (SECDEF)")
        XCTAssertFalse(app.upstreamCompat)
        XCTAssertEqual(app.services, o.services, "every optional service off by default")
        // A test's plaintext is for loopback only.
        setup.tls = .off
        XCTAssertEqual(CoreHubService.tlsMode(compat: false, setup: setup), .require)
        setup.loopback = true
        XCTAssertEqual(CoreHubService.tlsMode(compat: false, setup: setup), .off)
        XCTAssertEqual(CoreHubService.tlsMode(compat: true, setup: setup), .on)
        // The services round-trip through the preferences.
        CoreHubService.save(HubServices(health: true, inventory: false, identify: false, history: true,
                                        alarms: false, groups: false), to: setup.defaults)
        let back = CoreHubService.load(setup.defaults)
        XCTAssertTrue(back.health && back.history)
        XCTAssertFalse(back.inventory || back.identify || back.alarms || back.groups)
    }

    func testP2SWDefaultHubRequiresTlsAndRunsNoOptionalService() throws {
        let ad = P2SWAdvertiser()
        let service = CoreHubService(devices: .demo, setup: p2swSetup("default"), discovery: ad)
        service.setSharing(true)
        p2swWait(15, "the hub running") { service.isRunning }
        XCTAssertNil(service.lastError)
        let security = try XCTUnwrap(service.security)
        XCTAssertEqual(security.tls, .require)
        XCTAssertFalse(security.upstreamCompat)
        XCTAssertTrue(security.fingerprint?.hasPrefix("SHA256:") == true, security.fingerprint ?? "none")
        for cap in p2swOptionalCaps {
            XCTAssertFalse(security.services.contains(cap), "\(cap) runs on a default hub: \(security.services)")
        }
        XCTAssertEqual(service.statusForTest().security.tls, .require)
        // tls=1 in the TXT record (the hub's own rule).
        XCTAssertEqual(ad.started?.ad.tls, true)
        XCTAssertTrue(ad.started?.ad.txtData().range(of: Data("tls=1".utf8)) != nil)

        // A plaintext client is refused: the hub requires TLS.
        let url = "ws://127.0.0.1:\(service.boundPort ?? 0)/dspi/v1"
        let client = DspiCore()
        XCTAssertThrowsError(try p2swOffMain("a plaintext connect") {
            try client.linkConnect(url: url, clientName: "p2sw", appVersion: "0", token: "", expectedHubId: "")
        })
        client.linkClose()

        // The pairing link under the PIN carries the hub, the key and the PIN.
        service.allowNewClient()
        p2swWait(5, "a PIN") { service.activePIN != nil }
        if CoreHubService.primaryLANAddress() != nil {
            p2swWait(5, "the pairing link") { service.pairingLink != nil }
            let link = try XCTUnwrap(service.pairingLink)
            XCTAssertTrue(link.hasPrefix("dspi://pair?h=\(service.hubID ?? "")&a="), link)
            XCTAssertTrue(link.contains("&k=") && link.hasSuffix("&p=\(service.activePIN ?? "")"), link)
            XCTAssertNotNil(PairingLinkRow.qr(link))
        }
        service.cancelPairing()
        XCTAssertNil(service.pairingLink)
        service.setSharing(false)
        XCTAssertNil(service.security)
    }

    func testP2SWCompatAsksFirstAndServesBoth() throws {
        let ad = P2SWAdvertiser()
        let service = CoreHubService(devices: .demo, setup: p2swSetup("compat"), discovery: ad)
        service.setSharing(true)
        p2swWait(15, "the hub running") { service.isRunning }
        XCTAssertFalse(CoreHubService.securityEffects.upstreamCompat.isEmpty)

        service.setUpstreamCompat(true)
        XCTAssertTrue(service.compatPending, "asked first")
        XCTAssertFalse(service.upstreamCompat, "nothing changes before the yes")
        service.cancelUpstreamCompat()
        XCTAssertFalse(service.compatPending)
        XCTAssertEqual(service.statusForTest().security.tls, .require)

        service.setUpstreamCompat(true)
        service.confirmUpstreamCompat()
        XCTAssertTrue(service.upstreamCompat)
        p2swWait(15, "the restart in compatibility mode") {
            service.isRunning && service.security?.upstreamCompat == true
        }
        XCTAssertEqual(service.security?.tls, .on)
        XCTAssertEqual(ad.started?.ad.tls, false, "no tls=1 in the compatibility mode")
        XCTAssertNotNil(service.security?.fingerprint, "TLS is served too")
        // Plaintext gets in (an upstream client).
        let url = "ws://127.0.0.1:\(service.boundPort ?? 0)/dspi/v1"
        let client = DspiCore()
        let hello = try p2swOffMain("a plaintext connect") {
            try client.linkConnect(url: url, clientName: "p2sw", appVersion: "0", token: "", expectedHubId: "")
        }
        XCTAssertTrue(hello.needsPairing)
        client.linkClose()

        // Off again at once, no question.
        service.setUpstreamCompat(false)
        XCTAssertFalse(service.upstreamCompat)
        p2swWait(15, "the restart with TLS required") {
            service.isRunning && service.security?.tls == .require
        }
        service.setSharing(false)
    }

    /// The services on (plaintext loopback, for the core's test client): the
    /// families the Swift screens use answer through the core's calls.
    func testP2SWServicesServeTheClientFamilies() throws {
        let service = CoreHubService(devices: .demo, setup: p2swSetup("services", tls: .off),
                                     discovery: P2SWAdvertiser())
        service.services = HubServices(health: true, inventory: true, identify: true, history: true,
                                       alarms: true, groups: false)
        service.setSharing(true)
        p2swWait(15, "the hub running") { service.isRunning }
        let caps = try XCTUnwrap(service.security?.services)
        for cap in ["health", "inventory", "identify", "events", "alarms"] {
            XCTAssertTrue(caps.contains(cap), "\(cap) missing: \(caps)")
        }
        XCTAssertFalse(caps.contains("groups"))
        p2swWait(5, "the demo device") { service.sharedSerials.count == 1 }

        service.allowNewClient()
        p2swWait(5, "a PIN") { service.activePIN != nil }
        let pin = try XCTUnwrap(service.activePIN)
        let url = "ws://127.0.0.1:\(service.boundPort ?? 0)/dspi/v1"
        let client = DspiCore()
        let hubCaps = try p2swOffMain("connect, pair and open the device") { () -> HubCaps in
            _ = try client.linkConnect(url: url, clientName: "p2sw client", appVersion: "0", token: "",
                                       expectedHubId: "")
            _ = try client.linkPair(pin: pin)
            for _ in 0..<200 where client.linkDevices().isEmpty { Thread.sleep(forTimeInterval: 0.05) }
            let d = try XCTUnwrap(client.linkDevices().first)
            try client.linkSelectDevice(handle: d.handle, serial: d.serial)
            _ = try client.fetchAll()
            return client.hubCaps()
        }
        XCTAssertTrue(hubCaps.onHub && hubCaps.identify && hubCaps.events && hubCaps.alarms && hubCaps.hubSnapshots,
                      "\(hubCaps)")
        let identify = try p2swOffMain("identify") { try client.hubIdentify(seconds: 1, toneOutput: nil) }
        _ = identify
        let page = try p2swOffMain("the history") { try client.hubEventsRead(serial: "", afterId: 0, limit: 50) }
        XCTAssertFalse(page.records.isEmpty, "the attach is in the hub's history")
        _ = try p2swOffMain("the alarms") { try client.hubAlarms(serial: "", state: "active") }
        let merged = try p2swOffMain("the merged snapshots") { client.snapshotListMerged(serial: "") }
        XCTAssertTrue(merged.hubListed)
        client.linkClose()
        service.setSharing(false)
    }
}
#endif
