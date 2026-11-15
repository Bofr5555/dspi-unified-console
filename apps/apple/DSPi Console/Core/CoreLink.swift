//
//  CoreLink.swift
//  DSPi Console
//
//  WP SW6.  The console as a DSPi Link client through the Rust core: one
//  hub's session is the core's (`link_connect`, `link_pair`,
//  `link_select_device`; the core's supervisor keeps it up, reconnects with
//  backoff, binds the device again and re-reads it).  What only the app knows
//  stays here: the stored tokens (LinkTokenStore, the Keychain), the PIN
//  prompt (the Mac's modal, the iOS sheet), the device the user picked, and
//  the hub's address, which the app's Bonjour browser finds (HubBrowser: the
//  Local Network entitlement and `NSBonjourServices` belong to the app).
//
//  `CoreLink` drives one hub; `CoreBackedTransport(link: .link(CoreLink))`
//  is the `DeviceTransport` the composite routes a hub's devices to, so a
//  device on another machine's hub gets every "with a core attached" feature
//  (typed domains, undo, snapshots, guarded applies with the hub's backstop,
//  the analyser, the client read-only mode, the hub's Installer copy).
//  Swift's own client (LinkClient.swift, NetworkTransport.swift) is only the
//  opt-out now: `-CoreTransport NO` / DSPI_CORE_TRANSPORT=0.
//
//  Threads.  The session's calls block on the network (a first dial retries
//  for up to a minute, a pairing waits for the hub), so they run on this
//  link's own queues, never on main and never on the transport's coreQueue,
//  whose GETs must not wait behind a dial: `dialQueue` (connect, pair) and
//  `statusQueue` (status, devices, retry, close; none blocks on the network).
//  The core is thread-safe; the device's own calls (`link_select_device`, a
//  connect sequence) run on the transport's coreQueue as every device call
//  does.  The core's listener only queues a look at the status.  State here
//  is main-thread only.
//

import Foundation
import Combine
import os
import dspi_core

// MARK: - What the composite needs of one hub

/// One hub's transport, as `CompositeTransport` drives it: the core's
/// (`CoreBackedTransport` over a `CoreLink`) or, under the opt-out, Swift's
/// `NetworkTransport`.  Main thread.
protocol RemoteHubTransport: DeviceTransport {
    /// The composite's key for the hub (`DiscoveredHub.id`).
    var hubID: String { get }
    var hubName: String { get }
    /// No session, or the last one ended: a hub that is off the browse list
    /// as well is given up on.  A session dialling, waiting for a PIN or
    /// reconnecting is up.
    var isSessionDown: Bool { get }
    /// The connection failed: the owner asks the browser whether the hub moved.
    var onConnectionFailed: (() -> Void)? { get set }
    /// The Mac's modal PIN prompt: the PIN, or nil to give up.
    var pairingPrompt: ((String) -> String?)? { get set }
    /// The same question without blocking (the iOS sheet).
    var asyncPairingPrompt: ((String, @escaping (String?) -> Void) -> Void)? { get set }
    /// Open the session to the hub at `url`.
    func connect(to url: URL)
    /// The browser listed the hub at `url` (`reappeared`: it was absent).
    func hubSeen(at url: URL, reappeared: Bool)
    /// The browser's current entry for the hub (a typed address's `/info`).
    func hubUpdated(_ hub: DiscoveredHub)
    /// Drive the hub's device `serial`, binding when the hub lists it.
    func bind(serial: String)
}

extension RemoteHubTransport {
    func hubUpdated(_ hub: DiscoveredHub) {}
}

extension NetworkTransport: RemoteHubTransport {}

// MARK: - The session

final class CoreLink {
    /// The composite's key for the hub (`DiscoveredHub.id`; "manual:host:port"
    /// for a typed address).
    let hubID: String
    let hubName: String
    /// The hub's own id, the `expected_hub_id` of every dial: a token goes
    /// out only after the hub's hello named this hub, and another hub at the
    /// address fails the dial with `wrong_hub`, having seen no token.  ""
    /// while unknown (a typed address whose `/info` has not answered): then
    /// the hub that answers names itself, and that id is kept from then on.
    private(set) var expectedHubID: String
    let core: DspiCore
    private let tokens: LinkTokenStore
    private let clientName: String
    private let appVersion: String

    private static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "CoreLink")

    /// Blocking session calls: `link_connect`, `link_pair`.
    private let dialQueue = DispatchQueue(label: "com.dspi.core.link.dial", qos: .userInitiated)
    /// Non-blocking session calls: status, devices, retry, address, close.
    private let statusQueue = DispatchQueue(label: "com.dspi.core.link.status", qos: .userInitiated)

    /// The transport this session feeds (set by its init).  Main thread.
    weak var transport: CoreBackedTransport?

    // MARK: Main-thread state

    /// The address last dialled or seen.
    private(set) var url: URL?
    /// The app wants this hub's session (cleared by a disconnect or a
    /// cancelled PIN prompt).
    private(set) var wantConnection = false
    /// A `link_connect` is running.
    private(set) var dialing = false
    /// Something happened during a dial (the hub announced again, the user
    /// picked its device): if the dial fails, one more.
    private var redialAfterDial = false
    /// The core has a session for us (from a dial that succeeded until the
    /// core says it ended, or a close).
    private(set) var sessionLive = false
    /// Bumped by every dial and every close: an older call's outcome is
    /// dropped.
    private var epoch = 0
    /// The phase the last look saw.
    private(set) var phase: LinkPhase = .idle
    private(set) var lastStatus: dspi_core.LinkStatus?
    /// The hub's devices as the last look saw them.
    private(set) var devices: [LinkDevice] = []
    /// The device the user picked, by serial (kept across reconnects).
    private(set) var boundSerial: String?
    /// The hub's name for it, for the placeholder while it is not listed.
    private var boundName: String?
    /// The core is bound to `boundSerial` (a `link_select_device` succeeded);
    /// the core binds it again by itself after a reconnect.
    private(set) var attached = false
    /// A `link_select_device` is running.
    private var binding = false
    /// A look at the status is queued.
    private var followQueued = false
    /// The pairing prompt is up.
    private var pairingPromptShowing = false
    /// A `link_pair` is running: the status still says a PIN is needed.
    private var pairing = false
    /// A look found a reconnect under way: the next `.bound` is a rebind.
    private var sawReconnecting = false

    var onConnectionFailed: (() -> Void)?
    var pairingPrompt: ((String) -> String?)?
    var asyncPairingPrompt: ((String, @escaping (String?) -> Void) -> Void)?

    private var listener: LinkEvents?

    /// - Parameters:
    ///   - expectedHubID: the hub's id when known ("" otherwise).
    ///   - core: the core to drive (a new one by default).
    init(hubID: String, hubName: String, expectedHubID: String, tokens: LinkTokenStore,
         clientName: String, appVersion: String = CoreLink.appVersion, core: DspiCore = DspiCore()) {
        self.hubID = hubID
        self.hubName = hubName
        self.expectedHubID = expectedHubID.lowercased()
        self.tokens = tokens
        self.clientName = clientName
        self.appVersion = appVersion
        self.core = core
        let events = LinkEvents(self)
        listener = events
        statusQueue.async { core.setLinkListener(listener: events) }
    }

    deinit {
        let core = self.core
        statusQueue.async {
            core.setLinkListener(listener: nil)
            core.linkClose()
        }
    }

    static var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0"
    }

    /// The hub's own id for a browser entry: a browsed hub's id is the hub's
    /// (its TXT `hid`); a typed address's is what its `/info` said, if it has.
    static func expectedID(of hub: DiscoveredHub) -> String {
        hub.isManual ? (hub.infoHubID ?? "") : hub.id
    }

    /// A hub's transport through the core.
    static func transport(for hub: DiscoveredHub, tokens: LinkTokenStore, clientName: String) -> CoreBackedTransport {
        let link = CoreLink(hubID: hub.id, hubName: hub.name, expectedHubID: expectedID(of: hub),
                            tokens: tokens, clientName: clientName)
        return CoreBackedTransport(link: .link(link))
    }

    // MARK: Tokens

    /// The stored token: under the hub's id, else (a typed address) under the
    /// composite's key, where the opt-out's client stored it.
    private func storedToken() -> String {
        if !expectedHubID.isEmpty, let t = tokens.token(forHub: expectedHubID) { return t }
        return tokens.token(forHub: hubID) ?? ""
    }

    /// A token the hub issued for `id` (the hub's own id).
    private func store(token: String, forHub id: String) {
        guard !token.isEmpty else { return }
        let id = id.lowercased()
        if expectedHubID.isEmpty, !id.isEmpty { expectedHubID = id }
        tokens.setToken(token, forHub: id.isEmpty ? hubID : id)
        // A typed address has no id of its own until it answers: keep the
        // token under its key too, for the next launch's first dial.
        if hubID.lowercased() != id { tokens.setToken(token, forHub: hubID) }
    }

    private func forgetToken() {
        if !expectedHubID.isEmpty { tokens.removeToken(forHub: expectedHubID) }
        tokens.removeToken(forHub: hubID)
    }

    // MARK: Connecting

    /// Open the session to the hub at `url` (a user's pick, or the composite
    /// the first time a device of this hub is selected).
    func connect(to url: URL) {
        assert(Thread.isMainThread)
        self.url = url
        wantConnection = true
        guard !sessionLive else {
            // The core keeps the session up: a new address is its next dial.
            setURL(url)
            return
        }
        dial()
    }

    /// One `link_connect`, never two at once (each replaces the session).
    private func dial() {
        guard wantConnection, let url = url else { return }
        if dialing { redialAfterDial = true; return }
        dialing = true
        redialAfterDial = false
        epoch += 1
        let epoch = self.epoch
        let token = storedToken()
        let expected = expectedHubID
        let (core, name, version) = (self.core, clientName, appVersion)
        let address = url.absoluteString
        Self.log.info("dialling \(address, privacy: .public) (token: \(!token.isEmpty), expected hub: \(expected.isEmpty ? "-" : expected, privacy: .public))")
        dialQueue.async { [weak self] in
            let result: Result<LinkHello, DspiError>
            do {
                result = .success(try core.linkConnect(url: address, clientName: name, appVersion: version,
                                                       token: token, expectedHubId: expected))
            } catch {
                result = .failure(error as? DspiError ?? .Transfer(msg: String(describing: error)))
            }
            DispatchQueue.main.async { self?.dialed(result, epoch: epoch) }
        }
    }

    private func dialed(_ result: Result<LinkHello, DspiError>, epoch: Int) {
        guard epoch == self.epoch else { return }
        dialing = false
        guard wantConnection else { return }
        switch result {
        case .success(let hello):
            sessionLive = true
            if expectedHubID.isEmpty { expectedHubID = hello.hubId.lowercased() }
            Self.log.info("hub \(hello.hubName, privacy: .public) answered (pairing: \(hello.needsPairing), token refused: \(hello.tokenRefused))")
            transport?.linkError(nil)
            if hello.needsPairing {
                if hello.tokenRefused { forgetToken() }
                askPIN()
            }
            scheduleFollow()
        case .failure(let error):
            Self.log.error("dial failed: \(String(describing: error), privacy: .public)")
            transport?.linkError(Self.words(error))
            transport?.linkSessionEnded()
            onConnectionFailed?()
            if redialAfterDial { dial() }
        }
    }

    /// The hub's new address (`link_set_url`): the core dials there from now
    /// on, at once when reconnecting.
    private func setURL(_ url: URL) {
        let core = self.core
        let address = url.absoluteString
        statusQueue.async { core.linkSetUrl(url: address) }
    }

    /// The browser listed the hub at `url`.  A session that is up is the
    /// core's: a new address is handed to it, and a hub back on the list
    /// while reconnecting is tried at once.  With no session (the last dial
    /// failed) the hub is dialled again on news only: a new address, or the
    /// hub back after an absence.  Never on a timer.
    func hubSeen(at url: URL, reappeared: Bool) {
        let moved = self.url != url
        self.url = url
        guard wantConnection else { return }
        if sessionLive {
            if moved { setURL(url) }
            if reappeared, phase == .reconnecting { retryNow() }
            return
        }
        if moved || reappeared { dial() }
    }

    /// A typed address's `/info` named the hub: its id becomes the expected one.
    func hubUpdated(_ hub: DiscoveredHub) {
        guard hub.id == hubID, expectedHubID.isEmpty else { return }
        let id = Self.expectedID(of: hub)
        if !id.isEmpty { expectedHubID = id.lowercased() }
    }

    /// `link_retry_now`: reconnect now instead of waiting out the backoff (a
    /// no-op unless reconnecting).
    func retryNow() {
        let core = self.core
        statusQueue.async { core.linkRetryNow() }
        scheduleFollow()
    }

    /// Back in the foreground, or the user's Reconnect: the core's session
    /// tries at once (`link_retry_now`); with none, dial again.
    func resume() {
        if !wantConnection, boundSerial != nil { wantConnection = true }
        if sessionLive { retryNow() } else { dial() }
    }

    /// Leave the hub: the session closes (its slot goes back to the hub) and
    /// the core drops the device bound to it.  The selection stays, so
    /// picking the device again reconnects.
    func disconnect() {
        wantConnection = false
        closeSession()
    }

    private func closeSession() {
        epoch += 1
        dialing = false
        pairing = false
        transport?.linkPhaseChanged(.idle)
        sessionLive = false
        attached = false
        binding = false
        sawReconnecting = false
        phase = .idle
        let core = self.core
        statusQueue.async { core.linkClose() }
        transport?.linkSessionEnded()
    }

    // MARK: Pairing

    /// Ask for the hub's PIN (one prompt at a time).
    private func askPIN() {
        guard !pairingPromptShowing, !pairing else { return }
        if let ask = asyncPairingPrompt {
            pairingPromptShowing = true
            ask(hubName) { [weak self] pin in
                DispatchQueue.main.async {
                    self?.pairingPromptShowing = false
                    self?.pinAnswered(pin)
                }
            }
            return
        }
        guard let prompt = pairingPrompt else {
            transport?.linkError("This hub needs pairing.")
            return
        }
        pairingPromptShowing = true
        let pin = prompt(hubName)
        pairingPromptShowing = false
        pinAnswered(pin)
    }

    /// The PIN typed, or nil for Cancel (the connection is given up).
    private func pinAnswered(_ pin: String?) {
        guard let pin = pin else {
            disconnect()
            return
        }
        guard wantConnection else { return }
        pairing = true
        let epoch = self.epoch
        let core = self.core
        dialQueue.async { [weak self] in
            let result: Result<String, DspiError>
            do {
                result = .success(try core.linkPair(pin: pin))
            } catch {
                result = .failure(error as? DspiError ?? .Transfer(msg: String(describing: error)))
            }
            DispatchQueue.main.async { self?.paired(result, epoch: epoch) }
        }
    }

    private func paired(_ result: Result<String, DspiError>, epoch: Int) {
        guard epoch == self.epoch else { return }
        pairing = false
        guard wantConnection else { return }
        switch result {
        case .success(let token):
            // An open hub grants a session without a token.  The listener
            // stores a token too (it may come after a timeout); the same here.
            if !token.isEmpty { store(token: token, forHub: expectedHubID) }
            transport?.linkError(nil)
            scheduleFollow()
        case .failure(let error):
            transport?.linkError("Pairing failed: \(Self.words(error))")
            switch error {
            case .Link(let code, _) where code == "wrong_pin":
                askPIN()   // the hub still waits: ask again
            case .Disconnected:
                // No session any more: the next dial asks again.
                sessionLive = false
                transport?.linkSessionEnded()
                dial()
            default:
                break
            }
        }
    }

    /// The core's listener: a token the hub issued.  Main thread.
    fileprivate func tokenIssued(hubID id: String, token: String) {
        store(token: token, forHub: id)
    }

    // MARK: Following the core's session

    /// Look at the core's status and devices once more (coalesced).
    func scheduleFollow() {
        guard !followQueued else { return }
        followQueued = true
        let core = self.core
        statusQueue.async { [weak self] in
            let status = core.linkStatus()
            let devices = core.linkDevices()
            DispatchQueue.main.async {
                guard let self = self else { return }
                self.followQueued = false
                self.follow(status, devices)
            }
        }
    }

    private func follow(_ status: dspi_core.LinkStatus, _ devices: [LinkDevice]) {
        lastStatus = status
        // A dial in progress: the status still describes the session before it.
        guard sessionLive, !dialing else { return }
        let previous = phase
        phase = status.phase
        transport?.linkPhaseChanged(phase)
        if status.connected || !devices.isEmpty { self.devices = devices }
        publishDevices()
        switch status.phase {
        case .needsPairing:
            if status.tokenRefused { forgetToken() }
            askPIN()
        case .reconnecting:
            sawReconnecting = true
            transport?.linkError(Self.phaseWords(status))
        case .closed, .refused:
            // Ended for good (revoked, a refused handshake): a person is needed.
            sessionLive = false
            attached = false
            sawReconnecting = false
            transport?.linkError(status.closeReason.isEmpty ? Self.phaseWords(status) : status.closeReason)
            transport?.linkSessionEnded()
            onConnectionFailed?()
        case .bound:
            transport?.linkError(nil)
            if sawReconnecting || previous != .bound, attached {
                sawReconnecting = false
                // The core bound the device again and re-read it: the app
                // re-reads its state and starts the analyser again.
                transport?.linkRebound()
            }
        case .noDevice, .chooseDevice, .idle:
            break
        }
        resolveBinding()
    }

    /// The hub's devices as the device list.
    private func publishDevices() {
        transport?.linkDevicesChanged(devices.map(device(from:)))
        publishSelected()
    }

    private func device(from d: LinkDevice) -> DSPiDevice {
        DSPiDevice(serial: d.serial, locationID: 0,
                   hub: RemoteHubRef(hubID: hubID, hubName: hubName, handle: UInt8(truncatingIfNeeded: d.handle)),
                   remoteName: d.name.isEmpty ? nil : d.name)
    }

    /// The selection: the hub's entry, or a placeholder (handle 255) while
    /// the hub does not list the device, so the picker stays on it.
    private func publishSelected() {
        guard let serial = boundSerial else { return transport?.linkSelected(nil) ?? () }
        if let d = devices.first(where: { $0.serial == serial }) {
            boundName = d.name.isEmpty ? boundName : d.name
            transport?.linkSelected(device(from: d))
        } else {
            transport?.linkSelected(DSPiDevice(serial: serial, locationID: 0,
                                               hub: RemoteHubRef(hubID: hubID, hubName: hubName, handle: 255),
                                               remoteName: boundName))
        }
    }

    /// Drive `serial`: bound when the hub lists it online.  Picking it again
    /// after a failed dial or a cancelled PIN prompt is how a user tries
    /// again.
    func bind(serial: String) {
        if serial != boundSerial {
            boundName = nil
            if attached { attached = false }
        }
        boundSerial = serial
        publishSelected()
        if !wantConnection, let url = url {
            connect(to: url)
            return
        }
        if !sessionLive {
            if dialing { redialAfterDial = true } else { dial() }
            return
        }
        resolveBinding()
    }

    /// Bind the core to the picked device once the session is ready and the
    /// hub lists it online.
    private func resolveBinding() {
        guard sessionLive, !binding, !attached, let serial = boundSerial,
              let status = lastStatus, status.connected,
              phase == .chooseDevice || phase == .bound || phase == .noDevice,
              let d = devices.first(where: { $0.serial == serial }), Self.isOnline(d) else { return }
        binding = true
        let epoch = self.epoch
        transport?.linkSelect(handle: d.handle, serial: serial) { [weak self] error in
            guard let self = self, epoch == self.epoch else { return }
            self.binding = false
            if let error = error {
                Self.log.error("select \(serial, privacy: .public) failed: \(String(describing: error), privacy: .public)")
                self.transport?.linkError((error as? DspiError).map(Self.words) ?? String(describing: error))
            } else if self.boundSerial == serial {
                self.attached = true
            }
            self.scheduleFollow()
        }
    }

    /// The hub's device states a board answers in (spec 7.4; the core's
    /// `board_present`).
    static func isOnline(_ d: LinkDevice) -> Bool {
        d.state == "online" || d.state == "updating"
    }

    // MARK: Words

    /// A dial's or a pairing's failure as one line: the core's notice row
    /// for it, with the core's own words.
    static func words(_ error: DspiError) -> String {
        switch error {
        case .Link(let code, let msg):
            if code == "wrong_hub" {
                return "Another hub answers at this address; it was not sent this console's token. (\(msg))"
            }
            return msg
        case .Transfer(let msg), .InvalidArgument(let msg): return msg
        default: return ConsoleNotice(noticeFor(error: error)).line
        }
    }

    /// The core's words for a phase (`link_phase_text`).
    static func phaseWords(_ status: dspi_core.LinkStatus) -> String {
        let t = linkPhaseText(phase: status.phase, pinSource: status.pinSource).text
        guard status.phase == .reconnecting, !status.closeReason.isEmpty else { return t }
        return "\(t) (\(status.closeReason))"
    }
}

// MARK: - The core's listener

/// Called on the core's supervisor thread with no core lock held: only
/// queues a look on main.
private final class LinkEvents: LinkListener, @unchecked Sendable {
    private weak var link: CoreLink?
    init(_ link: CoreLink) { self.link = link }

    private func look() {
        DispatchQueue.main.async { [weak self] in self?.link?.scheduleFollow() }
    }

    func onLinkDevicesChanged() { look() }
    func onLinkClosed(code: Int32, reason: String) { look() }
    func onLinkStatusChanged() { look() }
    func onLinkTokenIssued(hubId: String, token: String) {
        DispatchQueue.main.async { [weak self] in self?.link?.tokenIssued(hubID: hubId, token: token) }
    }
}

// MARK: - The transport side

/// -- WP SW6 -- `CoreBackedTransport(link: .link(_))` is a hub's transport
/// for the composite.
extension CoreBackedTransport: RemoteHubTransport {
    private var linkOrDie: CoreLink {
        guard let l = linkSession else { preconditionFailure("not a DSPi Link transport") }
        return l
    }
    var hubID: String { linkOrDie.hubID }
    var hubName: String { linkOrDie.hubName }
    var isSessionDown: Bool { !(linkSession?.sessionLive ?? false) && !(linkSession?.dialing ?? false) }
    var onConnectionFailed: (() -> Void)? {
        get { linkSession?.onConnectionFailed }
        set { linkSession?.onConnectionFailed = newValue }
    }
    var pairingPrompt: ((String) -> String?)? {
        get { linkSession?.pairingPrompt }
        set { linkSession?.pairingPrompt = newValue }
    }
    var asyncPairingPrompt: ((String, @escaping (String?) -> Void) -> Void)? {
        get { linkSession?.asyncPairingPrompt }
        set { linkSession?.asyncPairingPrompt = newValue }
    }
    func connect(to url: URL) { linkSession?.connect(to: url) }
    func hubSeen(at url: URL, reappeared: Bool) { linkSession?.hubSeen(at: url, reappeared: reappeared) }
    func hubUpdated(_ hub: DiscoveredHub) { linkSession?.hubUpdated(hub) }
    func bind(serial: String) { linkSession?.bind(serial: serial) }
}
