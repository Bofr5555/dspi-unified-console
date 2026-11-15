//
//  CoreLinkSW6Tests.swift
//  DSPi ConsoleTests
//
//  WP SW6.  DSPi Link through the core (Core/CoreLink.swift): the core's
//  session over its own BSD sockets, driven by the app's tokens, PIN prompt
//  and picked device.
//
//  - Both targets: the default and the opt-out, the read-only flag on a
//    hub's transport, a hub's transport built by the factory.
//  - macOS: against the Mac's own Swift hub (LinkServer over a hub whose
//    device is Swift's demo FakeTransport), in process on loopback: pairing,
//    token reuse, a wrong PIN, a cancelled prompt, a revoked token, the
//    expected hub id, a lost link and the core's rebind, `link_retry_now` on
//    resume, `-ReadOnly`, and the composite's default.
//  - Opt-in (`TEST_RUNNER_DSPI_SW6_HUB=host:port`, both targets): against the
//    Rust hub (`uconsole --demo hub`), which has the analyser, the hub's
//    limits and lock (HUBADMIN, P1HL) and the hub's guard (P1HA).  Steps by
//    `TEST_RUNNER_DSPI_SW6_STEP`; the PIN of a pairing step in
//    `TEST_RUNNER_DSPI_SW6_PIN`.  Driven by the WP's host script
//    (work/wp-SW6/hw/hubrun.sh).
//
//  Links that must be lost run through `SW6Relay`, a TCP relay the test
//  drops or refuses at will.
//

import XCTest
import Combine
import Network
import dspi_core
@testable import DSPi_Console

// MARK: - Helpers

/// Spin the main run loop until `condition` holds (CoreLink is main-thread).
@discardableResult
private func sw6Wait(_ timeout: TimeInterval, _ what: String = "", file: StaticString = #filePath, line: UInt = #line,
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

/// A TCP relay on loopback in front of a hub: the test can drop every
/// connection through it (a lost link) and refuse new ones (an outage).
final class SW6Relay {
    private let listener: NWListener
    private let host: NWEndpoint.Host
    private let targetPort: NWEndpoint.Port
    private let queue = DispatchQueue(label: "sw6.relay")
    private let lock = NSLock()
    private var connections: [NWConnection] = []
    private var refusingValue = false
    private(set) var accepted = 0

    var refusing: Bool {
        get { lock.lock(); defer { lock.unlock() }; return refusingValue }
        set { lock.lock(); refusingValue = newValue; lock.unlock() }
    }

    init(host: String, port: UInt16) throws {
        self.host = NWEndpoint.Host(host)
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

    deinit { stop() }

    var port: UInt16 { listener.port?.rawValue ?? 0 }
    var url: URL { URL(string: "ws://127.0.0.1:\(port)/dspi/v1")! }

    func stop() {
        listener.cancel()
        dropAll()
    }

    /// End every connection through the relay at once.
    func dropAll() {
        lock.lock(); let all = connections; connections = []; lock.unlock()
        all.forEach { $0.forceCancel() }
    }

    private func accept(_ inbound: NWConnection) {
        if refusing { inbound.forceCancel(); return }
        let outbound = NWConnection(host: host, port: targetPort, using: .tcp)
        lock.lock(); connections += [inbound, outbound]; accepted += 1; lock.unlock()
        inbound.start(queue: queue)
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

/// A hub's transport through the core, with what a test watches.
private final class SW6Client {
    let link: CoreLink
    let transport: CoreBackedTransport
    let tokens: LinkTokenStore
    var prompts: [String] = []
    var answers: [String?] = []
    var failures = 0

    init(hubID: String, expectedHubID: String? = nil, tokens: LinkTokenStore, name: String = "SW6 test") {
        self.tokens = tokens
        link = CoreLink(hubID: hubID, hubName: "SW6 hub", expectedHubID: expectedHubID ?? hubID,
                        tokens: tokens, clientName: name)
        transport = CoreBackedTransport(link: .link(link))
        transport.onConnectionFailed = { [weak self] in self?.failures += 1 }
        transport.asyncPairingPrompt = { [weak self] name, done in
            guard let self = self else { return done(nil) }
            self.prompts.append(name)
            done(self.answers.isEmpty ? nil : self.answers.removeFirst())
        }
    }

    func open(_ url: URL, serial: String) {
        transport.connect(to: url)
        transport.bind(serial: serial)
    }

    func state() -> DspState? {
        if case .success(let s) = transport.readAll() { return s }
        return nil
    }

    func close() {
        transport.disconnect()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
    }
}

private func sw6Tokens() -> LinkTokenStore { LinkTokenStore(service: "sw6-test-\(UUID().uuidString)") }

// MARK: - Both targets

final class CoreLinkSW6Tests: XCTestCase {

    /// A hub's devices go through the core unless `-CoreTransport NO`.
    func testSW6DSPiLinkIsTheCoresByDefaultAndTheFlagOptsOut() throws {
        let d = UserDefaults.standard
        let saved = d.object(forKey: "CoreTransport")
        defer { if let saved = saved { d.set(saved, forKey: "CoreTransport") } else { d.removeObject(forKey: "CoreTransport") } }
        guard ProcessInfo.processInfo.environment["DSPI_CORE_TRANSPORT"] == nil else {
            throw XCTSkip("DSPI_CORE_TRANSPORT is set for this run")
        }
        d.removeObject(forKey: "CoreTransport")
        XCTAssertTrue(AppState.usesCoreLink, "the core by default")
        d.set(false, forKey: "CoreTransport")
        XCTAssertFalse(AppState.usesCoreLink, "-CoreTransport NO keeps Swift's Link client")
    }

    /// The factory gives a hub's transport over a CoreLink with the hub's id
    /// as the expected one; a typed address's id is its /info's.
    func testSW6TheFactoryBuildsACoreLinkWithTheExpectedHubID() {
        let tokens = sw6Tokens()
        let browsed = DiscoveredHub(id: "ABCDEF01-0000-4000-8000-000000000001", name: "Studio", kind: "console",
                                    authMode: "pin", deviceCount: 1, serials: ["S1"], path: "/dspi/v1", tls: false,
                                    host: "10.0.0.2", port: 11915, isManual: false)
        let t = CoreLink.transport(for: browsed, tokens: tokens, clientName: "Test")
        XCTAssertEqual(t.linkSession?.expectedHubID, browsed.id.lowercased())
        XCTAssertEqual(t.hubID, browsed.id)
        var typed = DiscoveredHub(id: "manual:10.0.0.3:11915", name: "10.0.0.3", kind: "unknown", authMode: "pin",
                                  deviceCount: 0, serials: [], path: "/dspi/v1", tls: false, host: "10.0.0.3",
                                  port: 11915, isManual: true)
        let m = CoreLink.transport(for: typed, tokens: tokens, clientName: "Test")
        XCTAssertEqual(m.linkSession?.expectedHubID, "", "a typed address has no id until /info answers")
        typed.infoHubID = "abcdef01-0000-4000-8000-000000000002"
        m.hubUpdated(typed)
        XCTAssertEqual(m.linkSession?.expectedHubID, "abcdef01-0000-4000-8000-000000000002")
        XCTAssertTrue(m.isSessionDown)
        XCTAssertFalse(m.isConnected)
    }

    /// -- WP ROMODE -- `-ReadOnly YES` covers a hub's transport now.
    func testSW6ReadOnlyCoversAHubsTransport() {
        let d = UserDefaults.standard
        let saved = d.object(forKey: "ReadOnly")
        defer { if let saved = saved { d.set(saved, forKey: "ReadOnly") } else { d.removeObject(forKey: "ReadOnly") } }
        d.set(true, forKey: "ReadOnly")
        let hub = DiscoveredHub(id: "abcdef01-0000-4000-8000-000000000003", name: "Studio", kind: "console",
                                authMode: "pin", deviceCount: 0, serials: [], path: "/dspi/v1", tls: false,
                                host: "10.0.0.2", port: 11915, isManual: false)
        let t = CoreLink.transport(for: hub, tokens: sw6Tokens(), clientName: "Test")
        XCTAssertTrue(t.clientReadOnly)
        XCTAssertEqual(try? t.call { $0.clientReadOnly() }.get(), true, "the core's own mode is on")
        d.set(false, forKey: "ReadOnly")
        XCTAssertFalse(CoreLink.transport(for: hub, tokens: sw6Tokens(), clientName: "Test").clientReadOnly)
    }
}

// MARK: - macOS: the Mac's own Swift hub, in process

#if os(macOS)
/// The Swift hub's device: Swift's demo FakeTransport.
private final class SW6FakeHubDevice: HubDevice {
    let fake: FakeTransport
    let info: HubDeviceInfo
    var generation: UInt64 { fake.generation }
    var isConnected: Bool { fake.isConnected }

    init(_ fake: FakeTransport) {
        self.fake = fake
        info = HubDeviceInfo(serial: FakeTransport.demoDevice.serial, platform: 1, firmware: "1.1.6", firmwareBeta: 0,
                             outputs: 9, inputs: 2, wireVersion: nil, link: .usb, boardName: "SW6 demo")
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
private final class SW6SwiftHub {
    let usb = CoreUsb(startMonitoring: false)
    let auth: LinkAuthStore
    let hub: LinkHub
    let server: LinkServer
    let fake: FakeTransport
    let port: Int

    init() throws {
        let auth = LinkAuthStore(storeURL: FileManager.default.temporaryDirectory
            .appendingPathComponent("sw6-hub-\(UUID().uuidString).json"))
        let policy = LinkPolicy.bundled ?? LinkPolicy.empty
        let hub = LinkHub(usb: usb, policy: policy, auth: auth)
        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        sw6Wait(5, "the demo device") { fake.isConnected }
        hub.attachDevice(SW6FakeHubDevice(fake))
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

    /// A client paired through `url` (the hub's own, or a relay's).
    fileprivate func pairedClient(_ tokens: LinkTokenStore, via url: URL? = nil) -> SW6Client {
        let c = SW6Client(hubID: hubID, tokens: tokens)
        c.answers = [auth.beginPairing()]
        c.open(url ?? self.url, serial: serial)
        sw6Wait(20, "pairing and the bind") { c.transport.isConnected }
        return c
    }
}

final class CoreLinkSW6SwiftHubTests: XCTestCase {

    /// A PIN hub: the prompt names the hub, the PIN pairs (role control), the
    /// token is stored under the hub's id, the device is bound and read; a
    /// second client with the same tokens connects with no prompt.
    func testSW6PairingStoresTheTokenAndTheNextConnectionReusesIt() throws {
        let hub = try SW6SwiftHub()
        let tokens = sw6Tokens()
        let c = hub.pairedClient(tokens)
        XCTAssertEqual(c.prompts, ["SW6 hub"])
        XCTAssertNotNil(tokens.token(forHub: hub.hubID), "the token is kept under the hub's id")
        XCTAssertEqual(c.link.lastStatus?.role, "control", "a PIN gives control")
        XCTAssertEqual(c.transport.selectedDevice?.serial, hub.serial)
        XCTAssertEqual(c.transport.selectedDevice?.hub?.hubID, hub.hubID)
        let state = try XCTUnwrap(c.state(), "the device reads through the hub")
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        XCTAssertFalse(c.transport.availableDevices.isEmpty)
        c.close()
        XCTAssertFalse(c.transport.isConnected)

        let again = SW6Client(hubID: hub.hubID, tokens: tokens)
        again.open(hub.url, serial: hub.serial)
        sw6Wait(15, "a connection on the stored token") { again.transport.isConnected }
        XCTAssertEqual(again.prompts, [], "the stored token pairs silently")
        again.close()
    }

    /// A wrong PIN asks again; Cancel gives the connection up.
    func testSW6AWrongPINAsksAgainAndCancelGivesUp() throws {
        let hub = try SW6SwiftHub()
        let pin = hub.auth.beginPairing()
        let wrong = pin == "000000" ? "111111" : "000000"
        let c = SW6Client(hubID: hub.hubID, tokens: sw6Tokens())
        c.answers = [wrong, pin]
        c.open(hub.url, serial: hub.serial)
        sw6Wait(20, "pairing on the second try") { c.transport.isConnected }
        XCTAssertEqual(c.prompts.count, 2, "asked again after the wrong PIN")
        c.close()

        _ = hub.auth.beginPairing()
        let gives = SW6Client(hubID: hub.hubID, tokens: sw6Tokens())
        gives.answers = [nil]
        gives.open(hub.url, serial: hub.serial)
        sw6Wait(15, "the prompt") { !gives.prompts.isEmpty }
        sw6Wait(5, "the session given up") { gives.transport.isSessionDown }
        XCTAssertFalse(gives.link.wantConnection)
        XCTAssertFalse(gives.transport.isConnected)
    }

    /// `expected_hub_id`: another hub at the address fails the dial with
    /// `wrong_hub`, says so, and asks the browser to look again; no PIN.
    func testSW6AnotherHubAtTheAddressIsRefused() throws {
        let hub = try SW6SwiftHub()
        let c = SW6Client(hubID: "0badbad0-0000-4000-8000-00000000beef", tokens: sw6Tokens())
        c.tokens.setToken("not-for-this-hub", forHub: "0badbad0-0000-4000-8000-00000000beef")
        c.answers = ["000000"]
        c.open(hub.url, serial: hub.serial)
        sw6Wait(20, "the refusal") { c.failures > 0 }
        XCTAssertEqual(c.prompts, [], "no pairing with another hub")
        XCTAssertTrue(c.transport.isSessionDown)
        XCTAssertFalse(c.transport.isConnected)
        XCTAssertTrue(c.transport.errorMessage?.contains("Another hub") ?? false, c.transport.errorMessage ?? "nil")
    }

    /// A lost link: the core reconnects with the same token, binds the
    /// device again and re-reads it; the transport says connected again,
    /// starts the analyser again and tells the view model to re-read.
    func testSW6ALostLinkReconnectsAndTheDeviceIsBoundAgain() throws {
        let hub = try SW6SwiftHub()
        let relay = try SW6Relay(host: "127.0.0.1", port: UInt16(hub.port))
        let c = hub.pairedClient(sw6Tokens(), via: relay.url)
        var resyncs = 0
        let watch = c.transport.addNotificationObserver { n in if n.eventID == 0x03 { resyncs += 1 } }
        let before = c.transport.reboundsForTest
        relay.dropAll()
        sw6Wait(10, "the loss seen") { c.link.phase == .reconnecting || c.transport.reboundsForTest > before }
        XCTAssertTrue(c.transport.isConnected, "a lost session is not a teardown: the last known state stays")
        sw6Wait(20, "the rebind") { c.transport.isConnected && c.link.phase == .bound && c.transport.reboundsForTest > before }
        sw6Wait(3, "the re-read") { resyncs > 0 }
        XCTAssertEqual(c.prompts.count, 1, "the same token: no prompt")
        XCTAssertGreaterThanOrEqual(relay.accepted, 2)
        XCTAssertNotNil(c.state(), "the device reads again")
        watch.cancel()
        c.close()
    }

    /// `link_retry_now` on resume (the app back in the foreground): during an
    /// outage the core backs off; once the network is back, resuming
    /// reconnects at once instead of after the wait.
    func testSW6ResumeRetriesNowInsteadOfWaitingOutTheBackoff() throws {
        let hub = try SW6SwiftHub()
        let relay = try SW6Relay(host: "127.0.0.1", port: UInt16(hub.port))
        let c = hub.pairedClient(sw6Tokens(), via: relay.url)
        relay.refusing = true
        relay.dropAll()
        // Wait until the next try is a while off (several failures in).
        sw6Wait(40, "a long backoff") {
            c.link.scheduleFollow()
            return (c.link.lastStatus?.reconnectInMs ?? 0) >= 5000
        }
        relay.refusing = false
        let start = Date()
        c.transport.reconnect()   // what the iOS app does on the way back
        sw6Wait(4, "a reconnect at once") { c.transport.isConnected && c.link.phase == .bound }
        XCTAssertLessThan(Date().timeIntervalSince(start), 4.5)
        c.close()
    }

    /// A revoked client: the hub closes it (4002), the core wants a PIN, the
    /// stored token goes, and the prompt comes again.
    func testSW6ARevokedTokenAsksForAPINAgain() throws {
        let hub = try SW6SwiftHub()
        let tokens = sw6Tokens()
        let c = hub.pairedClient(tokens)
        let cid = try XCTUnwrap(hub.auth.clients.first?.id)
        c.answers = [nil]
        hub.auth.revoke(cid: cid)
        hub.hub.closeSessions(forClient: cid)
        sw6Wait(20, "a new prompt") { c.prompts.count >= 2 || c.link.lastStatus?.phase == .needsPairing }
        sw6Wait(5, "the token gone") { tokens.token(forHub: hub.hubID) == nil }
    }

    /// -- WP ROMODE -- `-ReadOnly YES` on a device reached over DSPi Link:
    /// reads go on, writes are refused before anything is sent.
    func testSW6ReadOnlyRefusesWritesToAHubsDevice() throws {
        let hub = try SW6SwiftHub()
        let d = UserDefaults.standard
        let saved = d.object(forKey: "ReadOnly")
        defer { if let saved = saved { d.set(saved, forKey: "ReadOnly") } else { d.removeObject(forKey: "ReadOnly") } }
        d.set(true, forKey: "ReadOnly")
        let c = hub.pairedClient(sw6Tokens())
        XCTAssertTrue(c.transport.clientReadOnly)
        let before = try XCTUnwrap(c.state())
        let refused = c.transport.call { core -> Bool in
            do { try core.setPreamp(db: before.preampDb - 3); return false } catch DspiError.ReadOnly { return true } catch { return false }
        }
        XCTAssertEqual(try? refused.get(), true)
        XCTAssertEqual(c.state()?.preampDb, before.preampDb)
        c.close()
    }

    /// The composite's default: a hub's device picked from the list is
    /// driven through the core (a core state source, typed reads).
    func testSW6TheCompositeDrivesAHubsDeviceThroughTheCore() throws {
        let hub = try SW6SwiftHub()
        let tokens = sw6Tokens()
        // A local side with no device (the hub's demo has the local demo's serial).
        let composite = CompositeTransport(local: CoreBackedTransport(link: .document), tokens: tokens,
                                           makeRemote: { CoreLink.transport(for: $0, tokens: tokens, clientName: "SW6") })
        let pin = hub.auth.beginPairing()
        composite.asyncPairingPrompt = { _, done in done(pin) }
        composite.updateHubs([DiscoveredHub(id: hub.hubID, name: "SW6 hub", kind: "console", authMode: "pin",
                                            deviceCount: 1, serials: [hub.serial], path: "/dspi/v1", tls: false,
                                            host: "127.0.0.1", port: hub.port, isManual: false)])
        let device = try XCTUnwrap(composite.availableDevices.first { $0.serial == hub.serial && $0.hub != nil })
        composite.selectDevice(device)
        sw6Wait(20, "the hub's device connected") { composite.isConnected }
        XCTAssertTrue(composite.isRemoteActive)
        let source = try XCTUnwrap(composite.coreStateSource as? CoreBackedTransport, "through the core")
        XCTAssertNotNil(source.linkSession)
        let state = source.readState()
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        composite.disconnect()
    }
}
#endif

// MARK: - Opt-in: the Rust hub (`uconsole --demo hub`), both targets

/// `TEST_RUNNER_DSPI_SW6_HUB=host:port` (and `_STEP`, `_PIN`).
final class CoreLinkSW6RustHubTests: XCTestCase {
    private var env: [String: String] { ProcessInfo.processInfo.environment }

    private func hubAddress() throws -> (String, UInt16) {
        guard let a = env["DSPI_SW6_HUB"], let colon = a.lastIndex(of: ":"),
              let port = UInt16(a[a.index(after: colon)...]) else {
            throw XCTSkip("opt-in: TEST_RUNNER_DSPI_SW6_HUB=host:port (uconsole --demo hub)")
        }
        return (String(a[..<colon]), port)
    }

    private func step(_ name: String) throws {
        guard env["DSPI_SW6_STEP"] == name else { throw XCTSkip("step \(name) (TEST_RUNNER_DSPI_SW6_STEP)") }
    }

    /// The hub's id from its /info (the core's `link_fetch_info`).
    private func hubID(_ host: String, _ port: UInt16) throws -> String {
        try linkFetchInfo(host: host, port: port).hubId.lowercased()
    }

    /// The token of one paired role, kept across runs in a file: a Keychain
    /// item made by one ad-hoc signed build asks for the login password in
    /// the next (each build is another app to the Keychain).  Each run uses
    /// a Keychain service of its own, filled from the file.
    private func tokenFile(_ role: String) -> URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("sw6-rusthub-\(role).token")
    }

    private func tokens(_ role: String, hub id: String) -> LinkTokenStore {
        let t = LinkTokenStore(service: "sw6-rusthub-\(UUID().uuidString)")
        if let saved = try? String(contentsOf: tokenFile(role), encoding: .utf8), !saved.isEmpty {
            t.setToken(saved, forHub: id)
        }
        return t
    }

    private func keep(_ c: SW6Client, _ role: String, hub id: String) {
        if let token = c.tokens.token(forHub: id) {
            try? token.write(to: tokenFile(role), atomically: true, encoding: .utf8)
        }
    }

    private func client(_ role: String, url: URL? = nil) throws -> SW6Client {
        let (host, port) = try hubAddress()
        let id = try hubID(host, port)
        let c = SW6Client(hubID: id, tokens: tokens(role, hub: id), name: "SW6 \(role)")
        if let pin = env["DSPI_SW6_PIN"] { c.answers = [pin] }
        let target = url ?? URL(string: "ws://\(host):\(port)/dspi/v1")!
        c.transport.connect(to: target)
        sw6Wait(30, "the session") { c.link.lastStatus?.connected == true && !c.link.devices.isEmpty }
        let serial = try XCTUnwrap(c.link.devices.first?.serial)
        c.transport.bind(serial: serial)
        sw6Wait(30, "the bind") { c.transport.isConnected }
        keep(c, role, hub: id)
        return c
    }

    /// Step `pair-<role>`: pair with the PIN the hub showed and keep the
    /// token (`role <cid> admin` on the hub then makes the first an Admin).
    func testSW6RustHubPairs() throws {
        guard let s = env["DSPI_SW6_STEP"], s.hasPrefix("pair-") else { throw XCTSkip("step pair-<role>") }
        let role = String(s.dropFirst(5))
        let (host, port) = try hubAddress()
        let id = try hubID(host, port)
        try? FileManager.default.removeItem(at: tokenFile(role))
        let c = try client(role)
        XCTAssertEqual(c.prompts.count, 1, "paired with the PIN")
        XCTAssertNotNil(c.tokens.token(forHub: id))
        XCTAssertNotNil(c.state())
        print("SW6: paired \(role) as \(c.link.lastStatus?.role ?? "?")")
        c.close()
    }

    /// Step `admin`: HUBADMIN's Installer copy for an Admin and for Control,
    /// and the hub-held lock (`held_by` hub): the Admin sets the hub's PIN and
    /// a lock; Control sees the hub's copy read-only, the lock held by the
    /// hub, a covered write refused, and the hub's PIN unlocking it.
    func testSW6RustHubInstallerAdminAndControlAndTheHubLock() throws {
        try step("admin")
        let admin = try client("admin")
        XCTAssertEqual(admin.link.lastStatus?.role, "admin")
        let target = try admin.transport.call { $0.installerTarget() }.get()
        XCTAssertEqual(target.copy, "hub")
        XCTAssertTrue(target.editable, "an Admin edits the hub's copy")
        XCTAssertEqual(target.noticeKey, "installer.hub_copy")
        let pages = modeManifest().screens.map(\.id).filter { !modeManifest().lockedUser.screens.contains($0) }
        let doc = """
        {"schemaVersion":1,"meta":{"kind":"limits","name":"SW6"},"limits":{"lockedPages":\(try String(data: JSONSerialization.data(withJSONObject: pages), encoding: .utf8)!),"masterVolumeMaxDb":-6}}
        """
        _ = try admin.transport.call { try $0.hubPinSet(pin: "2468") }.get()
        let set = try admin.transport.call { try $0.hubLimitsSet(document: doc) }.get()
        XCTAssertTrue(set.state.pinSet)
        XCTAssertFalse(set.state.profileDocument.isEmpty)
        defer {
            _ = admin.transport.call { try $0.hubPinClear() }
            _ = admin.transport.call { try $0.hubLimitsClear() }
            admin.close()
        }

        let control = try client("control")
        XCTAssertEqual(control.link.lastStatus?.role, "control")
        let t2 = try control.transport.call { core -> InstallerTarget in
            _ = try? core.hubLimitsGet()
            return core.installerTarget()
        }.get()
        XCTAssertEqual(t2.copy, "hub")
        XCTAssertFalse(t2.editable, "Control reads the hub's copy only")
        XCTAssertEqual(t2.readOnlyKey, "installer.hub_role")
        let lock = try control.transport.call { try $0.installerLockRefresh() }.get()
        XCTAssertEqual(lock.heldBy, "hub", "one lock: the hub's")
        XCTAssertTrue(lock.locked)
        let refused = control.transport.call { core -> String in
            do { try core.setPreamp(db: -2); return "sent" } catch DspiError.InstallerLock(let code, _) { return code } catch { return "\(error)" }
        }
        XCTAssertEqual(try? refused.get(), "hub", "refused before sending, as the hub's lock")
        let unlocked = try control.transport.call { try $0.installerUnlock(pin: "2468") }.get()
        XCTAssertFalse(unlocked.locked, "the hub's PIN unlocks it")

        // The Swift Installer page (CoreInstaller) over both: the hub's copy,
        // Admin editable, Control read-only with the core's row; an Admin's
        // change reaches Control's page through HubAdminListener.
        let adminVM = DSPViewModel(transport: admin.transport)
        let adminPage = CoreInstaller(vm: adminVM, folder: nil)
        let controlVM = DSPViewModel(transport: control.transport)
        let controlPage = CoreInstaller(vm: controlVM, folder: nil)
        sw6Wait(20, "the admin's page") { adminPage.available && adminPage.hub != nil && !adminPage.busy }
        sw6Wait(20, "control's page") { controlPage.available && controlPage.hub != nil && !controlPage.busy }
        XCTAssertTrue(adminPage.hubCopy)
        XCTAssertTrue(adminPage.editable, "Admin edits the hub's copy")
        XCTAssertTrue(adminPage.pinSet, "the hub's PIN")
        XCTAssertFalse(adminPage.copyLine.isEmpty)
        XCTAssertTrue(controlPage.hubCopy)
        XCTAssertFalse(controlPage.editable, "Control: read-only")
        XCTAssertFalse(controlPage.readOnlyLine.isEmpty, "with the core's installer.hub_role row")
        XCTAssertEqual(controlPage.draft.value("masterVolumeMaxDb"), -6)
        adminPage.setLimit("masterVolumeMaxDb", -9)
        sw6Wait(20, "the admin's change on control's page (limits.changed)") {
            controlPage.draft.value("masterVolumeMaxDb") == -9
        }
        control.close()
    }

    /// Step `control`: the analyser after a reconnect (`rta_start` again),
    /// and the hub's guard with its backstop (P1HA): a guarded recall is the
    /// hub's too, Revert puts it back, and a console that vanishes is
    /// reverted by the hub.
    func testSW6RustHubAnalyserAfterAReconnectAndTheHubsGuard() throws {
        try step("control")
        let (host, port) = try hubAddress()
        let relay = try SW6Relay(host: host, port: port)
        let c = try client("control", url: relay.url)
        // The analyser: started as the app does at a connect (SW14).
        c.transport.perform { $0.rtaStart() }
        sw6Wait(10, "the analyser's caps") { (try? c.transport.call { $0.rtaGetCaps().supported }.get()) == true }
        let rebinds = c.transport.reboundsForTest
        relay.dropAll()
        sw6Wait(20, "the rebind") { c.transport.isConnected && c.transport.reboundsForTest > rebinds }
        sw6Wait(10, "the analyser again after the reconnect") {
            (try? c.transport.call { $0.rtaGetCaps().supported }.get()) == true
        }

        // The hub's guard.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("sw6-snap-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let original = try XCTUnwrap(c.state()).preampDb
        let snap = try c.transport.call { core -> SnapshotInfo in
            try core.snapshotSetDir(dir: dir.path)
            return try core.snapshotCapture(name: "SW6", note: "")
        }.get()
        _ = try c.transport.call { try $0.setPreamp(db: original - 4) }.get()
        let g = try c.transport.call { try $0.applyGuardedSnapshot(id: snap.id, timeoutS: 30) }.get()
        XCTAssertTrue(g.hub, "the hub runs the guard too (its backstop)")
        XCTAssertEqual(g.phase, .waiting)
        _ = try c.transport.call { try $0.setPreamp(db: original - 7) }.get()
        let r = try c.transport.call { try $0.applyRevert() }.get()
        XCTAssertEqual(r.phase, .reverted)
        XCTAssertEqual(c.state()?.preampDb ?? 0, original - 4, accuracy: 0.01, "Revert puts the capture back")

        // The backstop: a guard begun, a change made, then this console is gone.
        let begun = try c.transport.call { try $0.applyGuardBegin(label: "SW6", timeoutS: 30) }.get()
        XCTAssertTrue(begun.hub)
        _ = try c.transport.call { try $0.setPreamp(db: original - 9) }.get()
        relay.refusing = true
        relay.dropAll()
        sw6Wait(15, "the loss") { c.link.phase == .reconnecting }
        XCTAssertTrue(c.transport.isConnected, "a lost session is not a teardown: the last known state stays")
        RunLoop.main.run(until: Date().addingTimeInterval(2))
        relay.refusing = false
        c.transport.reconnect()
        sw6Wait(30, "back") { c.transport.isConnected && c.link.phase == .bound }
        sw6Wait(10, "the hub's revert") { abs((c.state()?.preampDb ?? 0) - (original - 4)) < 0.01 }
        let status = try c.transport.call { $0.applyPoll() }.get()
        print("SW6: guard after the loss: \(status.phase) \(status.message)")
        _ = c.transport.call { try $0.setPreamp(db: original) }
        c.close()
    }
}
