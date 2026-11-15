//
//  LinkBannerSW7Tests.swift
//  DSPi ConsoleTests
//
//  WP SW7.  The lost-connection banner (Core/LinkBannerView.swift) and the
//  owner's rule for a lost DSPi Link connection: "connection lost,
//  reconnecting" with the way out (`link_close`), the mirror stays readable,
//  writes fail fast and are never replayed.  The wording is CG10's.
//
//  - Both targets: the banner's entries from a session's phase, the core's
//    words and button, the way out, no host-device noun in the table.
//  - macOS: a fake-hub run, the Mac's own Swift hub (LinkServer over a hub
//    whose device is Swift's demo FakeTransport) on loopback behind SW6's TCP
//    relay, which the test drops and refuses at will (an outage).
//

import XCTest
import Combine
import SwiftUI
import Network
import dspi_core
@testable import DSPi_Console

// MARK: - Helpers

@discardableResult
private func sw7Wait(_ timeout: TimeInterval, _ what: String = "", file: StaticString = #filePath, line: UInt = #line,
                     _ condition: () -> Bool) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while !condition() {
        if Date() > deadline {
            if !what.isEmpty { XCTFail("timed out waiting for \(what)", file: file, line: line) }
            return false
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    }
    return true
}

private func sw7Settle(_ seconds: TimeInterval) {
    RunLoop.main.run(until: Date().addingTimeInterval(seconds))
}

/// Nouns for what the user is holding, which shared text never names (owner).
private let sw7HostDeviceNouns = ["phone", "iphone", "ipad", "tablet", "computer", "laptop", "desktop", "mac ", "pc "]

// MARK: - Both targets

final class LinkBannerSW7Tests: XCTestCase {

    /// A reconnecting session has an entry in the core's words, with the
    /// core's button; any other phase has none.
    func testSW7TheBannerSaysTheCoresWordsWithTheWayOut() {
        let center = LinkBannerCenter()
        let session = NSObject()
        let id = ObjectIdentifier(session)
        var closed = 0
        center.report(id, hubName: "Studio hub", phase: .bound, pinSource: "", closeReason: "") { closed += 1 }
        XCTAssertTrue(center.entries.isEmpty, "bound: no banner")

        center.report(id, hubName: "Studio hub", phase: .reconnecting, pinSource: "",
                      closeReason: "connection reset") { closed += 1 }
        XCTAssertEqual(center.entries.count, 1, "reconnecting: the banner at once")
        let e = try! XCTUnwrap(center.entries.first)
        let core = linkPhaseText(phase: .reconnecting, pinSource: "")
        XCTAssertEqual(e.words, core, "the core's entry (CG10)")
        XCTAssertEqual(e.words.key, "reconnecting")
        XCTAssertEqual(e.text, core.text)
        XCTAssertEqual(e.words.actionKey, "reconnecting.disconnect", "the way out is the core's button")
        XCTAssertEqual(e.action, core.action)
        XCTAssertEqual(e.detail, "Studio hub: connection reset", "the hub and the core's close_reason, as on Android")

        // A later look with the same words changes nothing; another reason does.
        center.report(id, hubName: "Studio hub", phase: .reconnecting, pinSource: "",
                      closeReason: "") { closed += 1 }
        XCTAssertEqual(center.entries.count, 1)
        XCTAssertEqual(center.entries.first?.detail, "Studio hub")

        // Bound again: gone, and nothing was closed.
        center.report(id, hubName: "Studio hub", phase: .bound, pinSource: "", closeReason: "") { closed += 1 }
        XCTAssertTrue(center.entries.isEmpty)
        XCTAssertEqual(closed, 0)

        // The button: the session's way out runs once, the entry goes at once.
        center.report(id, hubName: "Studio hub", phase: .reconnecting, pinSource: "", closeReason: "") { closed += 1 }
        center.disconnect(id)
        XCTAssertEqual(closed, 1)
        XCTAssertTrue(center.entries.isEmpty)
        center.disconnect(id)
        XCTAssertEqual(closed, 1, "a second press does nothing")
    }

    /// Two hubs lost: each has its own entry and its own way out; the end of
    /// one session takes only its entry.
    func testSW7EachLostSessionHasItsOwnEntry() {
        let center = LinkBannerCenter()
        let a = NSObject(), b = NSObject()
        var closedA = 0, closedB = 0
        center.report(ObjectIdentifier(a), hubName: "A", phase: .reconnecting, pinSource: "", closeReason: "") { closedA += 1 }
        center.report(ObjectIdentifier(b), hubName: "B", phase: .reconnecting, pinSource: "", closeReason: "") { closedB += 1 }
        XCTAssertEqual(center.entries.map(\.hubName), ["A", "B"])
        center.disconnect(ObjectIdentifier(b))
        XCTAssertEqual([closedA, closedB], [0, 1])
        XCTAssertEqual(center.entries.map(\.hubName), ["A"])
        center.clear(ObjectIdentifier(a))
        XCTAssertTrue(center.entries.isEmpty)
        XCTAssertEqual(closedA, 0, "a session that ended is not closed again")
    }

    /// CG10's table: every phase has text, placeholders resolved by the
    /// compile target, no host-device noun; the banner's words come from it.
    func testSW7TheWordingIsCG10sAndNamesNoHostDevice() {
        let table = linkPhaseTexts()
        XCTAssertFalse(table.isEmpty)
        for t in table {
            let all = ([t.text] + (t.action.map { [$0] } ?? [])).joined(separator: " ")
            XCTAssertFalse(t.text.isEmpty, t.key)
            XCTAssertFalse(all.contains("[["), "a placeholder left in \(t.key): \(all)")
            let lower = all.lowercased() + " "
            for noun in sw7HostDeviceNouns {
                XCTAssertFalse(lower.contains(noun), "\(t.key) names the host device (\(noun)): \(all)")
            }
        }
        let reconnecting = try! XCTUnwrap(table.first { $0.key == "reconnecting" })
        XCTAssertEqual(reconnecting.text, "Connection lost, reconnecting")
        XCTAssertEqual(reconnecting.action, "Disconnect")
        XCTAssertEqual(linkPhaseText(phase: .reconnecting, pinSource: ""), reconnecting)
    }

    /// The view builds for an entry (both targets) with the core's words.
    func testSW7TheViewShowsTheEntry() {
        let center = LinkBannerCenter()
        let s = NSObject()
        center.report(ObjectIdentifier(s), hubName: "Hub", phase: .reconnecting, pinSource: "", closeReason: "") {}
        let entry = try! XCTUnwrap(center.entries.first)
        let view = LinkBannerView(entry: entry) {}
        XCTAssertEqual(view.entry.text, linkPhaseText(phase: .reconnecting, pinSource: "").text)
        #if os(macOS)
        let host = NSHostingView(rootView: LinkBannerStack(center: center).frame(width: 600))
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(host.fittingSize.height, 0, "the banner takes room while shown")
        #else
        let host = UIHostingController(rootView: LinkBannerStack(center: center).frame(width: 390))
        XCTAssertGreaterThan(host.sizeThatFits(in: CGSize(width: 390, height: 1000)).height, 0,
                             "the banner takes room while shown")
        #endif
    }
}

#if os(macOS)
// MARK: - macOS: a fake-hub run

/// The Swift hub's device: Swift's demo FakeTransport.
private final class SW7FakeHubDevice: HubDevice {
    let fake: FakeTransport
    let info: HubDeviceInfo
    var generation: UInt64 { fake.generation }
    var isConnected: Bool { fake.isConnected }

    init(_ fake: FakeTransport) {
        self.fake = fake
        info = HubDeviceInfo(serial: FakeTransport.demoDevice.serial, platform: 1, firmware: "1.1.6", firmwareBeta: 0,
                             outputs: 9, inputs: 2, wireVersion: nil, link: .usb, boardName: "SW7 demo")
    }

    func execute(_ request: LinkCmdRequest) -> LinkCmdResponse {
        switch request.direction {
        case .get:
            switch fake.getControlResult(request: request.bRequest, value: request.wValue, index: request.wIndex,
                                         length: request.wLength) {
            case .success(let data): return LinkCmdResponse(tag: request.tag, status: .ok, payload: data)
            case .failure(let status): return LinkCmdResponse(tag: request.tag, status: status)
            }
        case .set:
            fake.sendControlRequest(request: request.bRequest, value: request.wValue, index: request.wIndex,
                                    data: request.payload)
            return LinkCmdResponse(tag: request.tag, status: .ok)
        }
    }
}

/// LinkServer on loopback over a hub whose device is the demo FakeTransport.
private final class SW7SwiftHub {
    let usb = CoreUsb(startMonitoring: false)
    let auth: LinkAuthStore
    let hub: LinkHub
    let server: LinkServer
    let fake: FakeTransport
    let port: Int

    init() throws {
        let auth = LinkAuthStore(storeURL: FileManager.default.temporaryDirectory
            .appendingPathComponent("sw7-hub-\(UUID().uuidString).json"))
        let policy = LinkPolicy.bundled ?? LinkPolicy.empty
        let hub = LinkHub(usb: usb, policy: policy, auth: auth)
        sw7Settle(0.01)
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        sw7Wait(5, "the demo device") { fake.isConnected }
        hub.attachDevice(SW7FakeHubDevice(fake))
        let server = LinkServer(hub: hub, auth: auth, policy: policy)
        try server.start(port: 0)
        guard let p = server.boundPort else { throw XCTSkip("the hub did not bind a port") }
        self.auth = auth
        self.hub = hub
        self.fake = fake
        self.server = server
        port = p
    }

    deinit { server.stop() }

    var hubID: String { auth.hubID.uuidString.lowercased() }
    var url: URL { URL(string: "ws://127.0.0.1:\(port)/dspi/v1")! }
    var serial: String { FakeTransport.demoDevice.serial }
}

/// A hub's transport through the core, its own banner, and a view model.
private final class SW7Client {
    let link: CoreLink
    let transport: CoreBackedTransport
    let banner = LinkBannerCenter()
    let tokens: LinkTokenStore
    var answers: [String?] = []

    init(hubID: String, tokens: LinkTokenStore) {
        self.tokens = tokens
        link = CoreLink(hubID: hubID, hubName: "SW7 hub", expectedHubID: hubID, tokens: tokens, clientName: "SW7 test")
        link.banner = banner
        transport = CoreBackedTransport(link: .link(link))
        transport.asyncPairingPrompt = { [weak self] _, done in
            guard let self = self else { return done(nil) }
            done(self.answers.isEmpty ? nil : self.answers.removeFirst())
        }
    }

    /// The device as it is now (a fresh read of every value).
    func deviceState() -> DspState? {
        if case .success(let s) = transport.readAll() { return s }
        return nil
    }
}

/// SW6's relay with one more mode: `forbidding` answers a new connection's
/// WebSocket upgrade with HTTP 403, which the core takes as a refusal that
/// needs a person (`.refused`, no retry).
private final class SW7Relay {
    private let listener: NWListener
    private let targetPort: NWEndpoint.Port
    private let queue = DispatchQueue(label: "sw7.relay")
    private let lock = NSLock()
    private var connections: [NWConnection] = []
    private var forbiddingValue = false
    private var acceptedValue = 0

    var forbidding: Bool {
        get { lock.lock(); defer { lock.unlock() }; return forbiddingValue }
        set { lock.lock(); forbiddingValue = newValue; lock.unlock() }
    }
    var accepted: Int { lock.lock(); defer { lock.unlock() }; return acceptedValue }

    init(port: UInt16) throws {
        targetPort = NWEndpoint.Port(rawValue: port)!
        let params = NWParameters.tcp
        params.requiredInterfaceType = .loopback
        listener = try NWListener(using: params, on: .any)
        let ready = DispatchSemaphore(value: 0)
        listener.stateUpdateHandler = { state in
            if case .ready = state { ready.signal() }
            if case .failed = state { ready.signal() }
        }
        listener.newConnectionHandler = { [weak self] c in self?.accept(c) }
        listener.start(queue: queue)
        _ = ready.wait(timeout: .now() + 5)
    }

    deinit { listener.cancel(); dropAll() }

    var url: URL { URL(string: "ws://127.0.0.1:\(listener.port?.rawValue ?? 0)/dspi/v1")! }

    func dropAll() {
        lock.lock(); let all = connections; connections = []; lock.unlock()
        all.forEach { $0.forceCancel() }
    }

    private func accept(_ inbound: NWConnection) {
        lock.lock(); acceptedValue += 1; let forbid = forbiddingValue; lock.unlock()
        inbound.start(queue: queue)
        if forbid {
            inbound.receive(minimumIncompleteLength: 1, maximumLength: 65536) { _, _, _, _ in
                let reply = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                inbound.send(content: Data(reply.utf8), completion: .contentProcessed { _ in inbound.cancel() })
            }
            return
        }
        let outbound = NWConnection(host: "127.0.0.1", port: targetPort, using: .tcp)
        lock.lock(); connections += [inbound, outbound]; lock.unlock()
        outbound.start(queue: queue)
        pipe(inbound, outbound)
        pipe(outbound, inbound)
    }

    private func pipe(_ from: NWConnection, _ to: NWConnection) {
        from.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, done, error in
            if let data = data, !data.isEmpty { to.send(content: data, completion: .contentProcessed { _ in }) }
            if done || error != nil {
                from.forceCancel(); to.forceCancel()
                return
            }
            self?.pipe(from, to)
        }
    }
}

final class LinkBannerSW7FakeHubTests: XCTestCase {

    private var keep: [AnyObject] = []

    override func tearDown() {
        keep.removeAll()
        super.tearDown()
    }

    /// A client paired through `url`, with a view model that has read the device.
    private func client(_ hub: SW7SwiftHub, _ tokens: LinkTokenStore, via url: URL) -> (SW7Client, DSPViewModel) {
        let c = SW7Client(hubID: hub.hubID, tokens: tokens)
        c.answers = [hub.auth.beginPairing()]
        let vm = DSPViewModel(transport: c.transport)
        c.transport.connect(to: url)
        c.transport.bind(serial: hub.serial)
        sw7Wait(30, "pairing, the bind and the first read") {
            vm.isDeviceConnected && vm.fullReadGeneration == c.transport.generation
        }
        sw7Settle(0.3)
        return (c, vm)
    }

    /// A write through the core on the transport's queue, and how long it took.
    private func timedWrite(_ c: SW7Client, preamp db: Float) -> (Result<Void, DspiError>, TimeInterval) {
        let start = Date()
        let r = c.transport.call { try $0.setPreamp(db: db) }
        return (r, Date().timeIntervalSince(start))
    }

    /// The owner's rule, the way out: the link is lost and stays down; the
    /// banner says so at once in the core's words with the hub's name; the
    /// view model keeps the last values; a write fails at once; Disconnect
    /// closes the session (`link_close`): the banner goes, nothing retries
    /// when the network is back, and the failed writes never reach the device.
    func testSW7ALostLinkShowsTheBannerAndDisconnectLeavesTheHub() throws {
        let hub = try SW7SwiftHub()
        let relay = try SW6Relay(host: "127.0.0.1", port: UInt16(hub.port))
        let tokens = LinkTokenStore(service: "sw7-test-\(UUID().uuidString)")
        let (c, vm) = client(hub, tokens, via: relay.url)
        keep += [c, vm, relay, hub]
        XCTAssertTrue(c.banner.entries.isEmpty, "bound: no banner")
        let before = try XCTUnwrap(c.deviceState()).preampDb
        let shownBefore = vm.preampDB

        relay.refusing = true
        relay.dropAll()
        sw7Wait(10, "the banner") { !c.banner.entries.isEmpty }
        let entry = try XCTUnwrap(c.banner.entries.first)
        let words = linkPhaseText(phase: .reconnecting, pinSource: "")
        XCTAssertEqual(entry.text, words.text, "CG10's text")
        XCTAssertEqual(entry.action, words.action, "CG10's way out")
        XCTAssertTrue(entry.detail.hasPrefix("SW7 hub"), entry.detail)
        XCTAssertEqual(c.link.phase, .reconnecting)

        // The mirror stays readable.
        XCTAssertTrue(vm.isDeviceConnected, "the last known state stays on screen")
        XCTAssertTrue(c.transport.isConnected)
        XCTAssertEqual(vm.preampDB, shownBefore)
        XCTAssertEqual(c.transport.readState().preampDb, before, "the core's mirror answers")

        // Writes fail fast: the core's own call, and the view model's.
        let (r, took) = timedWrite(c, preamp: before - 3)
        guard case .failure(let error) = r else { return XCTFail("a write went through while reconnecting") }
        XCTAssertEqual(error, .Disconnected, "\(error)")
        XCTAssertLessThan(took, 1.0, "at once, not after a wait")
        vm.setPreamp(before - 6)
        c.transport.coreQueue.sync {}

        // The way out.
        let accepted = relay.accepted
        c.banner.disconnect(entry.id)
        XCTAssertTrue(c.banner.entries.isEmpty, "the banner goes at once")
        XCTAssertFalse(c.link.wantConnection)
        XCTAssertFalse(c.link.sessionLive)
        sw7Wait(5, "the core's session closed") { c.link.core.linkStatus().phase != .reconnecting }
        XCTAssertFalse(c.link.core.linkStatus().connected)

        // Nothing retries once the network is back: the client is not dialling.
        relay.refusing = false
        sw7Settle(4)
        XCTAssertEqual(relay.accepted, accepted, "no retry after Disconnect")
        XCTAssertTrue(c.banner.entries.isEmpty)
        XCTAssertFalse(c.transport.isConnected)

        // Never replayed: the device has its value from before.
        let again = SW7Client(hubID: hub.hubID, tokens: tokens)
        keep.append(again)
        again.transport.connect(to: hub.url)
        again.transport.bind(serial: hub.serial)
        sw7Wait(20, "a second client on the stored token") { again.transport.isConnected && again.link.phase == .bound }
        XCTAssertEqual(try XCTUnwrap(again.deviceState()).preampDb, before, "the refused writes never reached the device")
        again.transport.disconnect()
        sw7Settle(0.2)
    }

    /// The owner's rule, the reconnect: while the core reconnects the banner
    /// shows and writes fail; when the link is back (the user's Reconnect is
    /// `link_retry_now`) the banner goes, the device is bound again and
    /// re-read, and the writes made meanwhile were not replayed.
    func testSW7TheBannerGoesOnTheReconnectAndNothingIsReplayed() throws {
        let hub = try SW7SwiftHub()
        let relay = try SW6Relay(host: "127.0.0.1", port: UInt16(hub.port))
        let tokens = LinkTokenStore(service: "sw7-test-\(UUID().uuidString)")
        let (c, vm) = client(hub, tokens, via: relay.url)
        keep += [c, vm, relay, hub]
        let before = try XCTUnwrap(c.deviceState()).preampDb
        let rebounds = c.transport.reboundsForTest

        relay.refusing = true
        relay.dropAll()
        sw7Wait(10, "the banner") { !c.banner.entries.isEmpty }
        XCTAssertTrue(vm.isDeviceConnected)
        let (r, took) = timedWrite(c, preamp: before - 3)
        if case .success = r { XCTFail("a write went through while reconnecting") }
        XCTAssertLessThan(took, 1.0)
        vm.setPreamp(before - 6)
        c.transport.coreQueue.sync {}

        relay.refusing = false
        c.transport.reconnect()   // `link_retry_now`
        sw7Wait(20, "the rebind") { c.link.phase == .bound && c.transport.reboundsForTest > rebounds }
        sw7Wait(5, "the banner gone") { c.banner.entries.isEmpty }
        XCTAssertTrue(c.transport.isConnected)
        XCTAssertEqual(try XCTUnwrap(c.deviceState()).preampDb, before, "nothing replayed after the reconnect")
        sw7Wait(5, "the view model re-read") { vm.preampDB.first == before }
        sw7Settle(1)
        XCTAssertEqual(try XCTUnwrap(c.deviceState()).preampDb, before, "nor a moment later")
        c.transport.disconnect()
        sw7Settle(0.2)
    }
    /// -- fixup (review batch 2 F2, the reviewer's probe) -- The user's
    /// Disconnect holds: back from the background the iOS app calls
    /// `transport.reconnect()` (IOSApp), which must not dial the hub again;
    /// picking the hub's device again does.
    func testSW7TheUsersDisconnectHoldsAcrossAForegroundResume() throws {
        let hub = try SW7SwiftHub()
        let relay = try SW6Relay(host: "127.0.0.1", port: UInt16(hub.port))
        let tokens = LinkTokenStore(service: "sw7-test-\(UUID().uuidString)")
        let (c, vm) = client(hub, tokens, via: relay.url)
        keep += [c, vm, relay, hub]
        relay.refusing = true
        relay.dropAll()
        sw7Wait(10, "the banner") { !c.banner.entries.isEmpty }
        let entry = try XCTUnwrap(c.banner.entries.first)
        c.banner.disconnect(entry.id)
        sw7Wait(5, "the core's session closed") { c.link.core.linkStatus().phase != .reconnecting }
        XCTAssertFalse(c.link.wantConnection)

        relay.refusing = false
        let accepted = relay.accepted
        c.transport.reconnect()   // what IOSApp does when the scene comes back from the background
        sw7Settle(4)
        XCTAssertEqual(relay.accepted, accepted, "the user's Disconnect holds across a foreground resume (no re-dial)")
        XCTAssertFalse(c.link.wantConnection, "wantConnection stays off after the user's Disconnect")
        XCTAssertFalse(c.transport.isConnected)
        XCTAssertTrue(c.banner.entries.isEmpty)

        // The user picks the hub's device again: that dials.
        c.transport.bind(serial: hub.serial)
        sw7Wait(20, "connected again on the user's pick") { c.transport.isConnected && c.link.phase == .bound }
        XCTAssertGreaterThan(relay.accepted, accepted)
        c.transport.disconnect()
        sw7Settle(0.2)
    }

    /// -- fixup (review batch 2 F2) -- An end that needs a person holds too:
    /// the hub refuses the reconnect's handshake (HTTP 403, `.refused`); a
    /// foreground resume dials nothing, no banner stays; the user's pick does.
    func testSW7ARefusedSessionIsNotDialledByAForegroundResume() throws {
        let hub = try SW7SwiftHub()
        let relay = try SW7Relay(port: UInt16(hub.port))
        let tokens = LinkTokenStore(service: "sw7-test-\(UUID().uuidString)")
        let (c, vm) = client(hub, tokens, via: relay.url)
        keep += [c, vm, relay, hub]
        relay.forbidding = true
        relay.dropAll()
        sw7Wait(20, "the refusal") { c.link.lastStatus?.phase == .refused && !c.link.sessionLive }
        XCTAssertTrue(c.banner.entries.isEmpty, "refused is not reconnecting: no banner")
        XCTAssertFalse(c.transport.isConnected)

        relay.forbidding = false
        let accepted = relay.accepted
        c.transport.reconnect()   // IOSApp, back from the background
        sw7Settle(4)
        XCTAssertEqual(relay.accepted, accepted, "a refused session is not dialled again by a resume")
        XCTAssertFalse(c.transport.isConnected)

        c.transport.bind(serial: hub.serial)
        sw7Wait(20, "connected again on the user's pick") { c.transport.isConnected && c.link.phase == .bound }
        XCTAssertGreaterThan(relay.accepted, accepted)
        c.transport.disconnect()
        sw7Settle(0.2)
    }
}
#endif
