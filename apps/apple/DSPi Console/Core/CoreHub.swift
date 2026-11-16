//
//  CoreHub.swift
//  DSPi Console
//
//  -- WP SW-M3 -- The Mac's DSPi Link hub, from the Rust core (`dspi-hub`
//  through its UniFFI wrapper, module `dspi_hub`): sharing this Mac's devices
//  on the network, as the Qt app does through the C ABI.  The Networking
//  settings page and the menu bar drive it (`HubSharingModel`).
//
//  The hub's rules are the hub's, unchanged: pairing and its window, roles,
//  limits, the installer lock, the guarded apply and its backstop, the
//  core-voltage rule.  Swift keeps what is the OS's: the Bonjour
//  advertisement (`LinkDiscovery`, mDNSResponder), the power assertions, the
//  status line's address, the preferences.
//
//  How the devices get there.  The hub runs only while sharing is on (HUB.md
//  section 10, R1).  Then it owns USB and the app's own core is its session 1:
//  `HubHost.attachApp` hands the core's USB to the hub (the core's device is
//  closed, claimed by the hub and reopened through it), and the hub's stop
//  gives it back.  Both run on the local transport's core queue
//  (`CoreBackedTransport.withCoreForHub`).  With `-LocalTransport demo` the
//  hub serves a demo device of its own instead.
//
//  Settings apply at a start: a new name or access mode restarts a running
//  hub, which disconnects the clients (they reconnect, except after a change
//  of access mode, which revokes every token: the page asks first when that
//  costs any paired client).  The port changes only while sharing is off.
//
//  Threads.  Hub calls run on `hubQueue` (or the core queue, above), never on
//  main; the published state is main-thread only.  The hub's events come on
//  its own thread and are posted to main.
//
//  The Swift hub (Link/LinkHub.swift, LinkServer, LinkService, ...) is the
//  opt-out: `-CoreTransport NO`, or `-CoreHub NO` (AppState.usesCoreHub).
//

#if os(macOS)
import Foundation
import Combine
import SystemConfiguration
import os
import dspi_core
import dspi_hub

// MARK: - What the settings page and the menu bar drive

/// A paired client as the Networking page shows it.
struct HubClientRow: Identifiable, Equatable {
    let id: Int
    var name: String
    var role: LinkRole
    var lastSeen: Date?
}

/// The sharing coordinator the Networking page and the menu bar drive: the
/// Rust hub's (`CoreHubService`) by default, the Swift hub's (`LinkService`)
/// under the opt-out.  Main thread.
@MainActor
protocol HubSharingModel: ObservableObject {
    var sharingEnabled: Bool { get }
    func setSharing(_ on: Bool)
    var isRunning: Bool { get }
    var lastError: String? { get }
    var hubName: String { get set }
    var authMode: LinkAuthMode { get set }
    var port: Int { get set }
    var preventSleepWhileConnected: Bool { get set }
    var clientRows: [HubClientRow] { get }
    var activePIN: String? { get }
    var pairingExpires: Date? { get }
    var sessionCount: Int { get }
    var listenAddress: String? { get }
    func allowNewClient()
    func cancelPairing()
    func refreshClients()
    func setRole(_ client: HubClientRow, _ role: LinkRole)
    func revoke(_ client: HubClientRow)
    /// An access-mode change waiting for the user's yes: how many paired
    /// clients it would forget.  nil: none waiting.
    var accessChangeCost: Int? { get }
    func confirmAccessChange()
    func cancelAccessChange()
}

/// The Swift hub's coordinator, the opt-out, as the page sees it.
extension LinkService: HubSharingModel {
    var clientRows: [HubClientRow] {
        clients.map { HubClientRow(id: $0.id, name: $0.name, role: $0.role, lastSeen: $0.lastSeen) }
    }
    func setRole(_ client: HubClientRow, _ role: LinkRole) {
        if let c = clients.first(where: { $0.id == client.id }) { setRole(c, role) }
    }
    func revoke(_ client: HubClientRow) {
        if let c = clients.first(where: { $0.id == client.id }) { revoke(c) }
    }
    /// The Swift hub keeps its tokens across a change of access mode.
    var accessChangeCost: Int? { nil }
    func confirmAccessChange() {}
    func cancelAccessChange() {}
}

/// The DNS-SD advertiser; a protocol so tests can watch it.
protocol HubAdvertising: AnyObject {
    func start(name: String, port: Int, ad: LinkAdvertisement)
    func updateTXT(_ ad: LinkAdvertisement)
    func stop()
}

extension LinkDiscovery: HubAdvertising {}

/// The words for a hub's refusal.
extension HubHostError {
    var sentence: String {
        switch self {
        case .Invalid(let msg), .Stopped(let msg), .NoDevice(let msg), .AlreadyAttached(let msg),
             .NoHandle(let msg), .Identity(let msg), .AppAttached(let msg), .Io(let msg):
            return msg
        case .Running: return "The hub is already running."
        case .NotRunning: return "The hub is not running."
        case .InCallback: return "Not possible from a hub callback."
        case .OpenAccess: return "This hub lets anyone in, so there is nothing to pair."
        }
    }
}

// MARK: - The service

@MainActor
final class CoreHubService: ObservableObject, HubSharingModel {

    /// What the hub serves.
    enum Devices {
        /// The app's own DSPis on USB: the hub owns the bus while it runs, and
        /// the app's core (this transport's) is its session 1.
        case app(CoreBackedTransport)
        /// A demo device of the hub's own (`-LocalTransport demo`, tests).
        case demo
    }

    /// Where the hub keeps its store (id, clients, device names), and how it
    /// listens.  The defaults are the app's.
    struct Setup {
        var storeDir: URL = CoreHubService.defaultStoreDir()
        /// Listen on loopback only (tests).
        var loopback = false
        /// Serve the web pages on the same port.
        var web = true
        /// With `Devices.app`, the hub finds and claims the bus's DSPis.  Off
        /// under XCTest unless the run opts in (TestHostGuard), as the app's
        /// own USB is.
        var usb = TestHostGuard.mayOpenDevices
        var defaults: UserDefaults = .standard
        // -- WP P2SW --
        /// The TLS mode instead of the app's (`require`, or `on` in the
        /// compatibility mode).  Only `.off` with `loopback`: the plaintext
        /// clients of the tests and of HB7's conformance run.
        var tls: HubTls? = nil
    }

    private let devices: Devices
    private let setup: Setup
    private let discovery: HubAdvertising
    let host = HubHost()
    private let hubQueue = DispatchQueue(label: "se.bofr.dspi.corehub", qos: .userInitiated)
    nonisolated private static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "link")

    /// Back to main from a hub or core queue.
    nonisolated private static func onMain(_ body: @escaping @MainActor () -> Void) {
        DispatchQueue.main.async { MainActor.assumeIsolated { body() } }
    }

    // Persisted preferences: the Swift hub's keys, so the choices carry over.
    @Published private(set) var sharingEnabled: Bool { didSet { setup.defaults.set(sharingEnabled, forKey: Keys.enabled) } }
    @Published var port: Int { didSet { setup.defaults.set(port, forKey: Keys.port) } }
    /// Holds an idle-sleep assertion for as long as sharing is on.
    @Published var preventSleepWhileConnected: Bool {
        didSet {
            setup.defaults.set(preventSleepWhileConnected, forKey: Keys.preventSleep)
            if isRunning { endPowerActivity(); beginPowerActivity() }
        }
    }

    // Live state, mirrored for the UI.
    @Published private(set) var isRunning = false
    @Published private(set) var lastError: String?
    /// The name the hub runs under.  A change restarts a running hub.
    @Published var hubName: String {
        didSet {
            guard hubName != oldValue else { return }
            if isRunning { restart() }
        }
    }
    /// The access mode.  A change that would forget paired clients waits for
    /// `confirmAccessChange` (`accessChangeCost`); a running hub restarts.
    @Published var authMode: LinkAuthMode {
        didSet {
            guard authMode != oldValue, !applyingAccess else { return }
            let target = authMode
            applyingAccess = true
            authMode = oldValue
            applyingAccess = false
            requestAccessChange(to: target)
        }
    }
    @Published private(set) var clientRows: [HubClientRow] = []
    @Published private(set) var activePIN: String?
    @Published private(set) var pairingExpires: Date?
    @Published private(set) var sessionCount = 0
    @Published private(set) var accessChangeCost: Int?
    /// The hub's id while it runs, or the stored one.
    @Published private(set) var hubID: String?
    /// The port it listens on while running.
    @Published private(set) var boundPort: Int?

    // -- WP P2SW -- the hub's security and its optional services
    /// The upstream compatibility mode (off by default): TLS and plaintext
    /// both, plaintext pairing allowed.  Turning it on asks first
    /// (`compatPending`, `confirmUpstreamCompat`).
    @Published private(set) var upstreamCompat: Bool {
        didSet { setup.defaults.set(upstreamCompat, forKey: Keys.upstreamCompat) }
    }
    /// Turning the compatibility mode on waits for the user's yes, after the
    /// warning that lists every effect (`securityEffects`).
    @Published private(set) var compatPending = false
    /// The optional services (every one off by default).  A change restarts a
    /// running hub.
    @Published var services: HubServices {
        didSet {
            guard services != oldValue else { return }
            Self.save(services, to: setup.defaults)
            if isRunning { restart() }
        }
    }
    /// What the running hub serves, TLS-wise; nil while stopped.
    @Published private(set) var security: HubSecurity?
    /// The pairing link (the QR code's) while a PIN is shown and TLS is served.
    @Published private(set) var pairingLink: String?

    /// What the compatibility mode and TLS off turn off: the hub's own lists.
    static let securityEffects = hubSecurityEffects()

    private var applyingAccess = false
    private var pendingAccess: LinkAuthMode?
    private var sessions: Set<UInt16> = []
    private var pinTimer: Timer?
    private var activityToken: NSObjectProtocol?
    /// Bumped by every start and stop (main): a late answer from an older run is dropped.
    private var run: UInt64 = 0
    private lazy var events = Events(self)

    private enum Keys {
        static let enabled = "LinkSharingEnabled"
        static let port = "LinkSharingPort"
        static let preventSleep = "LinkPreventSleep"
        // -- WP P2SW --
        static let upstreamCompat = "LinkHubUpstreamCompat"
        static let services = "LinkHubServices"
    }

    // -- WP P2SW --
    /// The services as stored: the names of those on.
    nonisolated static func load(_ defaults: UserDefaults) -> HubServices {
        let on = Set(defaults.stringArray(forKey: Keys.services) ?? [])
        return HubServices(health: on.contains("health"), inventory: on.contains("inventory"),
                           identify: on.contains("identify"), history: on.contains("history"),
                           alarms: on.contains("alarms"), groups: on.contains("groups"))
    }

    nonisolated static func save(_ s: HubServices, to defaults: UserDefaults) {
        let on: [(Bool, String)] = [(s.health, "health"), (s.inventory, "inventory"), (s.identify, "identify"),
                                    (s.history, "history"), (s.alarms, "alarms"), (s.groups, "groups")]
        defaults.set(on.filter(\.0).map(\.1), forKey: Keys.services)
    }

    /// The TLS mode a start asks for: `require`, `on` in the compatibility
    /// mode, or a test's `.off` on loopback.
    nonisolated static func tlsMode(compat: Bool, setup: Setup) -> HubTls {
        if compat { return .on }
        if setup.tls == .off { return setup.loopback ? .off : .require }
        return setup.tls ?? .require
    }

    /// The start options: the hub's defaults (TLS required, no optional
    /// service), then this app's choices.
    nonisolated static func startOptions(name: String, port: Int, access: LinkAuthMode, usb: Bool, app: Bool,
                                         compat: Bool, services: HubServices, setup: Setup,
                                         mdnsName: String?) -> HubStartOptions {
        var o = hubDefaultStartOptions(storeDir: setup.storeDir.path)
        o.name = name
        o.port = UInt16(clamping: port)
        o.access = access == .none ? .open : .pin
        o.usb = usb
        o.app = app
        o.loopback = setup.loopback
        o.web = setup.web
        o.mdnsName = mdnsName
        o.tls = tlsMode(compat: compat, setup: setup)
        o.upstreamCompat = compat
        o.services = services
        return o
    }

    /// The hub's store: a folder beside the Swift hub's `link-auth.json`.
    nonisolated static func defaultStoreDir() -> URL {
        LinkAuthStore.defaultURL().deletingLastPathComponent().appendingPathComponent("link-hub", isDirectory: true)
    }

    /// The stored hub id, with no hub running (nil before the first start).
    nonisolated static func savedHubID(storeDir: URL = defaultStoreDir()) -> String? {
        (try? hubSavedSettings(storeDir: storeDir.path))?.hubId
    }

    init(devices: Devices, setup: Setup = Setup(), discovery: HubAdvertising = LinkDiscovery()) {
        self.devices = devices
        self.setup = setup
        self.discovery = discovery
        let d = setup.defaults
        sharingEnabled = d.object(forKey: Keys.enabled) as? Bool ?? false
        port = d.object(forKey: Keys.port) as? Int ?? Int(LinkDiscovery.defaultPort)
        preventSleepWhileConnected = d.object(forKey: Keys.preventSleep) as? Bool ?? false
        upstreamCompat = d.bool(forKey: Keys.upstreamCompat)   // -- WP P2SW --
        services = Self.load(d)
        let saved = try? hubSavedSettings(storeDir: setup.storeDir.path)
        hubName = saved?.name ?? LinkAuthStore.defaultHubName()
        authMode = saved?.access == .open ? .none : .pin
        hubID = saved?.hubId
        clientRows = (saved?.clients ?? []).map(Self.row)
        _ = host.setListener(listener: events)
        if sharingEnabled { start() }
    }

    // MARK: Lifecycle

    func setSharing(_ on: Bool) { on ? start() : stop() }

    /// Turn sharing on: start the hub, put the devices on it, advertise, and
    /// hold the power assertions.  Persists the choice.
    func start() {
        guard !isRunning else { return }
        lastError = nil
        sharingEnabled = true
        run &+= 1
        let thisRun = run
        let isApp: Bool = { if case .app = devices { return true }; return false }()
        // -- WP P2SW -- from the hub's defaults: TLS required, no optional service
        let options = Self.startOptions(name: hubName, port: port, access: authMode, usb: isApp && setup.usb,
                                        app: isApp, compat: upstreamCompat, services: services, setup: setup,
                                        mdnsName: Self.localHostName())
        let host = self.host
        let finish: @Sendable (Result<HubStarted, Error>) -> Void = { [weak self] result in
            Self.onMain { self?.started(result, run: thisRun) }
        }
        switch devices {
        case .app(let transport):
            transport.withCoreForHub { core in
                do {
                    let started = try host.start(options: options)
                    do {
                        try host.attachApp(core: core)
                    } catch {
                        CoreHubService.log.error("the app's core did not attach to the hub: \(String(describing: error), privacy: .public)")
                        _ = host.stop()
                        finish(.failure(error))
                        return false
                    }
                    finish(.success(started))
                    return true
                } catch {
                    finish(.failure(error))
                    return nil
                }
            }
        case .demo:
            hubQueue.async {
                do {
                    let started = try host.start(options: options)
                    _ = try host.attachDemo()
                    finish(.success(started))
                } catch {
                    _ = host.stop()
                    finish(.failure(error))
                }
            }
        }
    }

    private func started(_ result: Result<HubStarted, Error>, run thisRun: UInt64) {
        guard thisRun == run else { return }
        switch result {
        case .success(let started):
            // The name first: a rename of a running hub restarts it.
            if started.name != hubName { hubName = started.name }
            isRunning = true
            hubID = started.hubId
            boundPort = Int(started.port)
            security = started.security   // -- WP P2SW --
            if started.tokensRevoked > 0 {
                Self.log.notice("Link hub: the change of access mode forgot \(started.tokensRevoked) paired clients")
            }
            discovery.start(name: started.name, port: Int(started.port), ad: advertisement())
            beginPowerActivity()
            refresh()
            Self.log.notice("Link hub (core) listening on port \(started.port, privacy: .public)")
        case .failure(let error):
            isRunning = false
            sharingEnabled = false
            boundPort = nil
            let words = (error as? HubHostError)?.sentence ?? String(describing: error)
            lastError = "Could not start on port \(port): \(words)"
            Self.log.error("Link hub (core) failed to start on port \(self.port, privacy: .public): \(words, privacy: .public)")
        }
    }

    /// Turn sharing off: stop advertising and the hub (clients are told,
    /// 4003), give the devices back, drop the assertions.  Persists the choice.
    func stop() {
        sharingEnabled = false
        stopHub(then: nil)
    }

    private func stopHub(then next: (@MainActor () -> Void)?) {
        run &+= 1
        discovery.stop()
        endPowerActivity()
        clearPairing()
        isRunning = false
        boundPort = nil
        security = nil   // -- WP P2SW --
        sessions = []
        sessionCount = 0
        let host = self.host
        let done: @Sendable () -> Void = { Self.onMain { next?() } }
        switch devices {
        case .app(let transport):
            transport.withCoreForHub { _ in
                _ = host.stop()
                done()
                return false
            }
        case .demo:
            hubQueue.async {
                _ = host.stop()
                done()
            }
        }
        Self.log.notice("Link hub (core) stopped")
    }

    /// Apply new settings: stop, then start again.
    private func restart() {
        guard isRunning else { return }
        stopHub { [weak self] in
            guard let self = self, self.sharingEnabled else { return }
            self.start()
        }
        sharingEnabled = true
    }

    // MARK: Access mode

    private func requestAccessChange(to mode: LinkAuthMode) {
        let saved = try? hubSavedSettings(storeDir: setup.storeDir.path)
        let cost = mode == .none ? Int(saved?.tokensRevokedIfOpen ?? 0) : Int(saved?.tokensRevokedIfPin ?? 0)
        // Running, the store's clients are the live list.
        let live = isRunning ? (mode == .none ? clientRows.count : 0) : cost
        if max(cost, live) > 0 {
            pendingAccess = mode
            accessChangeCost = max(cost, live)
        } else {
            applyAccess(mode)
        }
    }

    func confirmAccessChange() {
        guard let mode = pendingAccess else { return }
        pendingAccess = nil
        accessChangeCost = nil
        applyAccess(mode)
    }

    func cancelAccessChange() {
        pendingAccess = nil
        accessChangeCost = nil
    }

    private func applyAccess(_ mode: LinkAuthMode) {
        applyingAccess = true
        authMode = mode
        applyingAccess = false
        if mode == .none { clearPairing() }
        if isRunning { restart() }
    }

    // MARK: Pairing and clients

    /// Show a PIN and accept pairing attempts while the hub's window is open.
    func allowNewClient() {
        guard isRunning else { return }
        let host = self.host
        hubQueue.async { [weak self] in
            let result = Result { try host.beginPairing() }
            Self.onMain {
                guard let self = self else { return }
                switch result {
                case .success(let pairing):
                    self.activePIN = pairing.pin
                    self.pairingExpires = Date().addingTimeInterval(TimeInterval(pairing.expiresInMs) / 1000)
                    self.pinTimer?.invalidate()
                    self.pinTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
                        Task { @MainActor in self?.refresh() }
                    }
                case .failure(let error):
                    self.lastError = (error as? HubHostError)?.sentence ?? String(describing: error)
                }
            }
        }
    }

    func cancelPairing() {
        let host = self.host
        hubQueue.async { _ = host.cancelPairing() }
        clearPairing()
    }

    private func clearPairing() {
        activePIN = nil
        pairingLink = nil   // -- WP P2SW --
        pairingExpires = nil
        pinTimer?.invalidate()
        pinTimer = nil
    }

    func setRole(_ client: HubClientRow, _ role: LinkRole) {
        let host = self.host
        let hubRole: HubClientRole = role == .admin ? .admin : role == .control ? .control : .viewer
        hubQueue.async { [weak self] in
            _ = host.setRole(cid: UInt32(clamping: client.id), role: hubRole)
            Self.onMain { self?.refresh() }
        }
    }

    func revoke(_ client: HubClientRow) {
        let host = self.host
        hubQueue.async { [weak self] in
            _ = host.revoke(cid: UInt32(clamping: client.id))
            Self.onMain { self?.refresh() }
        }
    }

    func refreshClients() { refresh() }

    /// Read the hub's clients, status and devices again (off main), then
    /// publish them: the client list, the pairing window, the advertisement.
    func refresh() {
        let host = self.host
        let storeDir = setup.storeDir.path
        let thisRun = run
        let pin = activePIN   // -- WP P2SW -- for the pairing link
        hubQueue.async { [weak self] in
            let status = host.status()
            let clients = status.running ? ((try? host.clients()) ?? [])
                                         : ((try? hubSavedSettings(storeDir: storeDir))?.clients ?? [])
            let shared = host.devices()
            let link = pin.flatMap { pin in
                Self.primaryLANAddress().flatMap { host.pairingLink(host: $0, pin: pin) }
            }
            Self.onMain {
                guard let self = self, thisRun == self.run else { return }
                // -- WP P2SW --
                if status.running, self.security != status.security { self.security = status.security }
                if self.activePIN != nil, self.activePIN == pin, self.pairingLink != link { self.pairingLink = link }
                let rows = clients.map(Self.row)
                if rows != self.clientRows { self.clientRows = rows }
                if self.activePIN != nil, !status.pairingOpen { self.clearPairing() }
                self.sharedSerials = shared.filter(\.online).map(\.serial)
                if self.isRunning { self.discovery.updateTXT(self.advertisement()) }
            }
        }
    }

    /// The serials of the devices the hub serves now.
    private(set) var sharedSerials: [String] = []

    private static func row(_ c: HubClient) -> HubClientRow {
        HubClientRow(id: Int(c.cid), name: c.name,
                     role: c.role == .admin ? .admin : c.role == .control ? .control : .viewer,
                     lastSeen: c.lastSeen.map { Date(timeIntervalSince1970: TimeInterval($0)) })
    }

    // MARK: Events

    /// The hub's events, on its own thread: posted to main.
    private final class Events: HubHostListener, @unchecked Sendable {
        private weak var service: CoreHubService?
        init(_ service: CoreHubService) { self.service = service }
        func onHubEvent(event: HubHostEvent) {
            CoreHubService.onMain { [weak self] in self?.service?.handle(event) }
        }
    }

    private func handle(_ event: HubHostEvent) {
        switch event {
        case .sessionOpened(let id):
            sessions.insert(id)
            sessionCount = sessions.count
            refresh()   // also how a finished pairing shows
        case .sessionClosed(let id):
            sessions.remove(id)
            sessionCount = sessions.count
            refresh()
        case .deviceOnline, .deviceOffline, .deviceRemoved:
            refresh()
        case .stopped:
            break
        case .tlsKey(let change, _):
            // -- WP P2SW -- the key's fingerprint is shown: read it again
            Self.log.notice("Link hub (core): TLS key \(String(describing: change), privacy: .public)")
            refresh()
        }
    }

    // MARK: Advertisement and status

    private func advertisement() -> LinkAdvertisement {
        // -- WP P2SW -- `tls=1` by the hub's own rule (TLS served, not in compat)
        LinkAdvertisement(hubID: hubID ?? "", auth: authMode.rawValue,
                          deviceCount: sharedSerials.count, serials: sharedSerials,
                          tls: security?.advertisesTls ?? false, web: setup.web)
    }

    // MARK: Security  -- WP P2SW --

    /// Turn the upstream compatibility mode on (after the warning: it waits
    /// for `confirmUpstreamCompat`) or off (at once).  A running hub restarts.
    func setUpstreamCompat(_ on: Bool) {
        if on {
            if !upstreamCompat { compatPending = true }
        } else {
            compatPending = false
            guard upstreamCompat else { return }
            upstreamCompat = false
            if isRunning { restart() }
        }
    }

    /// The user's yes to the warning.
    func confirmUpstreamCompat() {
        guard compatPending else { return }
        compatPending = false
        upstreamCompat = true
        if isRunning { restart() }
    }

    func cancelUpstreamCompat() { compatPending = false }

    /// One optional service, for the settings page (which does not import
    /// dspi_hub: only this file drives the hub).
    enum OptionalService: String, CaseIterable, Identifiable {
        case health, inventory, identify, history, alarms, groups
        var id: String { rawValue }

        var title: String { rawValue.prefix(1).uppercased() + rawValue.dropFirst() }

        var detail: String {
            switch self {
            case .health: return "CPU and temperature of each device"
            case .inventory: return "Every DSPi this hub has seen"
            case .identify: return "Lets clients make a device blink"
            case .history: return "A shared event log of each device"
            case .alarms: return "Offline, restarts, clipping, temperature"
            case .groups: return "Named groups of devices"
            }
        }

        var key: WritableKeyPath<HubServices, Bool> {
            switch self {
            case .health: return \.health
            case .inventory: return \.inventory
            case .identify: return \.identify
            case .history: return \.history
            case .alarms: return \.alarms
            case .groups: return \.groups
            }
        }
    }

    func isOn(_ s: OptionalService) -> Bool { services[keyPath: s.key] }
    func set(_ s: OptionalService, _ on: Bool) { services[keyPath: s.key] = on }

    /// The TLS mode as served, in words ("Not running" while stopped).
    var tlsWords: String {
        guard isRunning, let s = security else { return "Not running" }
        switch s.tls {
        case .require: return "Encrypted (TLS required)"
        case .on: return s.upstreamCompat ? "Encrypted and unencrypted (compatibility mode)" : "Encrypted and unencrypted"
        case .off: return "Not encrypted"
        }
    }

    /// The key's fingerprint as clients show it, while TLS is served.
    var fingerprint: String? { security?.fingerprint }
    /// The running hub is in the compatibility mode.
    var servesCompat: Bool { security?.upstreamCompat == true }
    /// What the compatibility mode turns off, one sentence each.
    static var compatEffects: [String] { securityEffects.upstreamCompat }

    /// The hub's own page, `https://` while TLS is served (the hub's own
    /// certificate: the browser asks once).
    var hubPageURL: String? {
        guard isRunning, setup.web, let ip = Self.primaryLANAddress() else { return nil }
        let scheme = (security?.tls ?? .off) == .off ? "http" : "https"
        return "\(scheme)://\(ip):\(boundPort ?? port)/"
    }

    /// The address a client on the LAN would type, best-effort.
    var listenAddress: String? {
        guard isRunning, let ip = Self.primaryLANAddress() else { return nil }
        return "\(ip):\(boundPort ?? port)"
    }

    /// This Mac's `.local` name, for the hub's `Origin` check on its web pages.
    nonisolated static func localHostName() -> String? {
        guard let name = SCDynamicStoreCopyLocalHostName(nil) as String?, !name.isEmpty else { return nil }
        return name + ".local"
    }

    /// First non-loopback IPv4 address on an up interface, for the status line.
    nonisolated static func primaryLANAddress() -> String? {
        var address: String?
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0, let first = ifaddr else { return nil }
        defer { freeifaddrs(ifaddr) }
        var ptr: UnsafeMutablePointer<ifaddrs>? = first
        while let p = ptr {
            let flags = Int32(p.pointee.ifa_flags)
            if (flags & (IFF_UP | IFF_RUNNING)) == (IFF_UP | IFF_RUNNING), (flags & IFF_LOOPBACK) == 0,
               let sa = p.pointee.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) {
                var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                if getnameinfo(sa, socklen_t(sa.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                    let ip = String(cString: host)
                    if ip.hasPrefix("192.168.") || ip.hasPrefix("10.") || ip.hasPrefix("172.") { address = ip; break }
                    if address == nil { address = ip }
                }
            }
            ptr = p.pointee.ifa_next
        }
        return address
    }

    // MARK: Power assertions

    /// Keep the app out of App Nap while it is serving; hold off idle sleep
    /// only when asked.
    private func beginPowerActivity() {
        guard activityToken == nil else { return }
        var options: ProcessInfo.ActivityOptions = [.userInitiatedAllowingIdleSystemSleep]
        if preventSleepWhileConnected { options.insert(.idleSystemSleepDisabled) }
        activityToken = ProcessInfo.processInfo.beginActivity(
            options: options, reason: "Sharing DSPi devices on the local network")
    }

    private func endPowerActivity() {
        if let token = activityToken {
            ProcessInfo.processInfo.endActivity(token)
            activityToken = nil
        }
    }

    // MARK: Tests

    /// Tests only: the keep-awake activity is held (sharing is on).
    var holdsPowerActivityForTest: Bool { activityToken != nil }

    /// Tests only: the hub's PIN window and client list, read now (off main).
    nonisolated func statusForTest() -> HubHostStatus { host.status() }
}
#endif
