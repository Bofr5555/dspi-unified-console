//
//  CoreBackedTransport.swift
//  DSPi Console
//
//  WP SW4.  `DeviceTransport` over the Rust core (docs/SWIFT-SEAMS.md 4.1,
//  5.1): the command layer's existing SETs and GETs go out as the core's raw
//  commands (CG4), its notifications come back through the core's raw tap
//  (CG23), and the device lock is the core's (CG24).  So the console shares
//  the core's one link while the domains still speak opcodes; SW8-SW14 move
//  them to the typed API one domain at a time.
//
//  The core reaches the device one of two ways:
//  * the demo device: the core's own `FakeDevice` (`openDemo`);
//  * a host link: another `DeviceTransport` carries the bytes (CG26's
//    `HostTransport`).  The core follows that transport's connection: it
//    opens when the transport connects and closes when it drops.  This is
//    how the Mac's core reaches its USB device through the in-process hub's
//    session 1 until SW-M3 (SWIFT-SEAMS 5.4), and how the tests run the
//    existing wire tests over Swift's `FakeTransport`.
//
//  Behind a launch flag: `-CoreTransport YES`, or DSPI_CORE_TRANSPORT=1 in
//  the environment (`TEST_RUNNER_DSPI_CORE_TRANSPORT=1 xcodebuild test`).
//
//  Threads.  Every core call runs on `coreQueue`, one serial queue: SETs
//  `async`, GETs and the lock `sync`, so a GET sees every earlier SET, as with
//  every transport before.  Nothing calls the core on the main thread except
//  `hostNotify`, which only queues a packet.  The core's listeners only
//  schedule work on `coreQueue`.
//

import Foundation
import Combine
import os
import dspi_core
import dspi_constants

final class CoreBackedTransport: DeviceTransport {

    // MARK: - The launch flag

    /// `-CoreTransport YES` (UserDefaults) or DSPI_CORE_TRANSPORT=1.
    static var isEnabled: Bool {
        if ProcessInfo.processInfo.environment["DSPI_CORE_TRANSPORT"] == "1" { return true }
        return UserDefaults.standard.bool(forKey: "CoreTransport")
    }

    /// What carries the core's link to the device.
    enum Link {
        /// The core's built-in demo device, offered as `FakeTransport.demoDevice`.
        case demo
        /// Another transport carries the bytes; `kind` sets the core's echo
        /// and bulk rules for it.
        case host(any DeviceTransport, HostKind)
    }

    /// The kind of a host link, as the core sees it.
    enum HostKind {
        /// A simulated device (Swift's `FakeTransport`): USB's rules.
        case fake
        /// A DSPi Link hub session (the Mac's `HubTransport`): our writes are
        /// the notifications whose origin is the transport's session.
        case hub
    }

    // MARK: - DeviceTransport state

    let session: LinkSessionID
    var generation: UInt64 { stateLock.lock(); defer { stateLock.unlock() }; return localGeneration }

    private let connectedSubject = CurrentValueSubject<Bool, Never>(false)
    private let devicesSubject: CurrentValueSubject<[DSPiDevice], Never>
    private let selectedSubject = CurrentValueSubject<DSPiDevice?, Never>(nil)
    private let errorSubject = CurrentValueSubject<String?, Never>(nil)
    private let fanout = NotificationFanout()

    var isConnected: Bool { connectedSubject.value }
    var isConnectedPublisher: AnyPublisher<Bool, Never> { connectedSubject.eraseToAnyPublisher() }
    var availableDevices: [DSPiDevice] { host?.availableDevices ?? devicesSubject.value }
    var availableDevicesPublisher: AnyPublisher<[DSPiDevice], Never> {
        host?.availableDevicesPublisher ?? devicesSubject.eraseToAnyPublisher()
    }
    var selectedDevice: DSPiDevice? { host?.selectedDevice ?? selectedSubject.value }
    var selectedDevicePublisher: AnyPublisher<DSPiDevice?, Never> {
        host?.selectedDevicePublisher ?? selectedSubject.eraseToAnyPublisher()
    }
    var errorMessage: String? { errorSubject.value }
    var errorMessagePublisher: AnyPublisher<String?, Never> { errorSubject.eraseToAnyPublisher() }

    // MARK: - The core and its queue

    /// The serial queue of every core call (SWIFT-SEAMS 5.1).
    let coreQueue = DispatchQueue(label: "com.dspi.core", qos: .userInitiated)
    private static let onCoreQueue = DispatchSpecificKey<Bool>()
    private let core: DspiCore
    private let host: (any DeviceTransport)?
    private let bridge: HostBridge?
    private var subscriptions: Set<AnyCancellable> = []
    private var notifyListener: Listener?

    /// Guards the generation and the drain flag.
    private let stateLock = NSLock()
    /// The `generation` this transport reports: bumped at once when the app
    /// opens or closes the device, so work issued before the switch is
    /// dropped even while the core is still connecting (as `FakeTransport`
    /// and `USBDevice` do), and when the core replaces the connection itself.
    private var localGeneration: UInt64 = 0
    /// The core's connection generation after the last open or close this
    /// transport made; any other value means the core changed it (coreQueue).
    private var knownCoreGeneration: UInt64 = 0
    private var drainScheduled = false

    private static let log = Logger(subsystem: "WeebLabs.DSPi-Console", category: "CoreBackedTransport")
    #if DEBUG
    private let mainGetLock = NSLock()
    private var mainGets: Set<UInt8> = []
    #endif

    /// - Parameters:
    ///   - link: what carries the core's link to the device.
    ///   - autoSelect: open the demo device straight away (demo only; a host
    ///     link follows its transport's own selection).
    ///   - core: the core to drive; a new one by default.
    init(link: Link, autoSelect: Bool = false, core: DspiCore = DspiCore()) {
        self.core = core
        coreQueue.setSpecific(key: CoreBackedTransport.onCoreQueue, value: true)
        switch link {
        case .demo:
            host = nil
            bridge = nil
            session = 1
            devicesSubject = CurrentValueSubject([FakeTransport.demoDevice])
        case .host(let transport, let kind):
            host = transport
            bridge = HostBridge(transport, kind: kind)
            session = kind == .hub ? transport.session : 1
            devicesSubject = CurrentValueSubject([])
        }

        let listener = Listener { [weak self] in self?.scheduleDrain() }
        notifyListener = listener
        coreQueue.async { [core] in
            core.notifyTapSet(enabled: true)
            core.setNotifyListener(listener: listener)
        }

        if let host = host, let bridge = bridge {
            // The device's packets go to the core's reader.  `hostNotify`
            // only queues them, so it may run on main.
            host.addNotificationObserver { [core] note in
                core.hostNotify(packet: note.packet, origin: bridge.origin(of: note))
            }
            .store(in: &subscriptions)
            // The core follows the host's connection.  The send happens on
            // the caller's thread; the core's open waits its turn on coreQueue.
            host.isConnectedPublisher
                .removeDuplicates()
                .sink { [weak self] connected in self?.hostConnectionChanged(connected) }
                .store(in: &subscriptions)
        } else if autoSelect {
            DispatchQueue.main.async { [weak self] in self?.selectDevice(FakeTransport.demoDevice) }
        }
    }

    deinit {
        let core = self.core
        coreQueue.async {
            core.setNotifyListener(listener: nil)
            core.disconnect()
        }
    }

    // MARK: - Connection

    func selectDevice(_ device: DSPiDevice) {
        if let host = host { host.selectDevice(device); return }
        guard device == FakeTransport.demoDevice else { return }
        bumpGeneration()
        selectedSubject.send(device)
        coreQueue.async { [weak self] in self?.openDemo() }
    }

    func reconnect() {
        if let host = host { host.reconnect(); return }
        guard let device = selectedDevice else { return }
        publishConnected(false)
        selectDevice(device)
    }

    func disconnect() {
        if let host = host { host.disconnect(); return }
        bumpGeneration()
        selectedSubject.send(nil)
        coreQueue.async { [weak self] in self?.close() }
    }

    /// A no-op: the core ends a dead link itself and says so with
    /// `NOTIFY_DISCONNECTED` (SWIFT-SEAMS 4.1).
    func markDisconnected() {}

    private func hostConnectionChanged(_ connected: Bool) {
        bumpGeneration()
        coreQueue.async { [weak self] in
            guard let self = self else { return }
            if connected { self.openHost() } else { self.close() }
        }
    }

    // On coreQueue.
    private func openDemo() {
        do {
            try onCore { try $0.openDemo() }
            opened(error: nil)
        } catch {
            opened(error: error)
        }
    }

    // On coreQueue.
    private func openHost() {
        guard let host = host, let bridge = bridge else { return }
        bridge.resetBulkCache()
        let serial = host.selectedDevice?.serial ?? ""
        do {
            try onCore { try $0.openHost(transport: bridge, serial: serial) }
            opened(error: nil)
        } catch {
            opened(error: error)
        }
    }

    // On coreQueue.
    private func opened(error: Error?) {
        let connected = onCore { $0.isConnected() }
        let generation = onCore { $0.connectionGeneration() }
        stateLock.lock(); knownCoreGeneration = generation; stateLock.unlock()
        if let error = error { Self.log.error("open failed: \(String(describing: error), privacy: .public)") }
        DispatchQueue.main.async {
            self.errorSubject.send(error.map(CoreBackedTransport.describe))
            self.publishConnected(connected)
        }
    }

    // On coreQueue.
    private func close() {
        onCore { $0.disconnect() }
        stateLock.lock(); knownCoreGeneration = 0; stateLock.unlock()
        DispatchQueue.main.async { self.publishConnected(false) }
    }

    /// On main.  Only a change: the view model treats every `false` as a
    /// disconnect and clears what it read.
    private func publishConnected(_ connected: Bool) {
        if connectedSubject.value != connected { connectedSubject.send(connected) }
    }

    private func bumpGeneration() {
        stateLock.lock(); localGeneration &+= 1; stateLock.unlock()
    }

    // MARK: - Notifications

    func addNotificationObserver(_ handler: @escaping (LinkNotification) -> Void) -> AnyCancellable {
        fanout.add(handler)
    }

    /// The core's batching: at most one drain per 30 ms (`NotifyListener`).
    private func scheduleDrain() {
        stateLock.lock()
        let already = drainScheduled
        drainScheduled = true
        stateLock.unlock()
        guard !already else { return }
        coreQueue.asyncAfter(deadline: .now() + .milliseconds(30)) { [weak self] in self?.drain() }
    }

    // On coreQueue.  Apply the queued packets to the core's mirror, hand the
    // tap's copies to the observers, and follow the connection.
    private func drain() {
        stateLock.lock(); drainScheduled = false; stateLock.unlock()
        let result = onCore { $0.processNotifications() }
        let packets = onCore { $0.notifyTapDrain() }
        for raw in packets {
            // SWIFT-SEAMS 4.1: the echo filter's verdict is the origin.
            let origin: LinkSessionID = raw.own ? session : 0
            let at = Date(timeIntervalSince1970: TimeInterval(raw.receivedMs) / 1000)
            fanout.publish(LinkNotification(packet: raw.packet, origin: origin, receivedAt: at))
        }
        let connected = onCore { $0.isConnected() }
        let coreGeneration = onCore { $0.connectionGeneration() }
        stateLock.lock()
        if coreGeneration != knownCoreGeneration {
            // The core reopened, re-bound or closed the link by itself.
            knownCoreGeneration = coreGeneration
            localGeneration &+= 1
        }
        stateLock.unlock()
        if result.flags & DspiConstants.NOTIFY_DISCONNECTED != 0 || connected != isConnected {
            DispatchQueue.main.async { self.publishConnected(connected) }
        }
    }

    // MARK: - SET (host to device)

    /// The core picks the interface, so `index` is dropped.
    func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
        coreQueue.async { [weak self] in
            guard let self = self else { return }
            do {
                try self.onCore { try $0.rawSet(opcode: request, value: value, data: data) }
            } catch {
                let status = CoreBackedTransport.status(for: error)
                if status == .locked { DeviceLock.notice.post() }
                Self.log.info("SET 0x\(String(format: "%02X", request), privacy: .public) wValue=\(value) failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    // MARK: - GET (device to host)

    func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, LinkStatus> {
        #if DEBUG
        if Thread.isMainThread { noteGetFromMain(request) }
        #endif
        return onCoreSync {
            do {
                return .success(try self.onCore { try $0.rawGet(opcode: request, value: value, len: length) })
            } catch {
                let status = CoreBackedTransport.status(for: error)
                if status == .locked { DeviceLock.notice.post() }
                return .failure(status)
            }
        }
    }

    #if DEBUG
    /// SWIFT-SEAMS 5.1: a GET from main still works, as it did over USB, and
    /// is logged once per opcode so each domain WP can clear its own.
    private func noteGetFromMain(_ request: UInt8) {
        mainGetLock.lock(); let first = mainGets.insert(request).inserted; mainGetLock.unlock()
        if first {
            Self.log.debug("GET 0x\(String(format: "%02X", request), privacy: .public) from the main thread")
        }
    }
    #endif

    // MARK: - Device lock (CG24)

    func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool {
        let ms = UInt32(max(0.001, min(DeviceLock.maxTimeout, timeout)) * 1000)
        return onCoreSync {
            do {
                try self.onCore { try $0.deviceLockAcquire(reason: reason, timeoutMs: ms) }
                return true
            } catch {
                Self.log.info("device lock refused: \(String(describing: error), privacy: .public)")
                return false
            }
        }
    }

    /// Gives back only the lock `acquireDeviceLock` took (owner, 2026-11-07):
    /// an IR learn window or a bootloader restart ends on its own exits.
    func releaseDeviceLock() {
        coreQueue.async { [weak self] in self?.onCore { $0.deviceLockRelease() } }
    }

    // MARK: - Errors

    /// `DspiError` as the status the command layer handles (SWIFT-SEAMS 4.1,
    /// review c4g nit 4).
    static func status(for error: Error) -> LinkStatus {
        guard let error = error as? DspiError else { return .error }
        switch error {
        case .Disconnected: return .noDevice
        case .Locked: return .locked
        case .Denied: return .denied
        // A read-only connection refuses writes, as MIDITransport's
        // read-only mode does.
        case .ReadOnly: return .denied
        case .Refused, .InvalidArgument: return .error
        case .Unsupported: return .unsupported
        case .Transfer: return .timeout
        case .Link(let code, _):
            switch code {
            case "rate_limited": return .rateLimited
            case "timeout": return .timeout
            default: return .noDevice
            }
        default: return .error
        }
    }

    private static func describe(_ error: Error) -> String {
        guard let error = error as? DspiError else { return String(describing: error) }
        switch error {
        case .Transfer(let msg), .InvalidArgument(let msg): return msg
        case .Link(_, let msg): return msg
        default: return String(describing: error)
        }
    }

    // MARK: - Queue helpers

    /// Every core call goes through here: on `coreQueue`, never main.
    @discardableResult
    private func onCore<T>(_ body: (DspiCore) throws -> T) rethrows -> T {
        assert(DispatchQueue.getSpecific(key: CoreBackedTransport.onCoreQueue) == true,
               "a core call off coreQueue")
        return try body(core)
    }

    /// Run `body` on `coreQueue` and wait for it (inline when already there).
    private func onCoreSync<T>(_ body: () -> T) -> T {
        if DispatchQueue.getSpecific(key: CoreBackedTransport.onCoreQueue) == true { return body() }
        return coreQueue.sync(execute: body)
    }
}

// MARK: - The core's notification listener

/// Called on the core's reader thread: it only schedules the drain.
private final class Listener: NotifyListener, @unchecked Sendable {
    private let onNotify: () -> Void
    init(_ onNotify: @escaping () -> Void) { self.onNotify = onNotify }
    func onNotifications() { onNotify() }
}

// MARK: - A DeviceTransport as the core's HostTransport (CG26)

/// Carries the core's control transfers over another `DeviceTransport`.
/// Called on `coreQueue` (or the core's analyser worker) with the core's lock
/// held, so it never calls back into the core.
final class HostBridge: HostTransport, @unchecked Sendable {
    private let transport: any DeviceTransport
    private let hostKind: CoreBackedTransport.HostKind

    /// wIndex of the vendor interface.  The core does not carry one, so every
    /// transfer uses the interface the firmware's vendor requests go to.
    static let vendorInterface: UInt16 = EXT_WINDEX

    private static let getAllParamsChunk: UInt8 = 0xA2
    private let cacheLock = NSLock()
    private var bulkCache: Data?

    init(_ transport: any DeviceTransport, kind: CoreBackedTransport.HostKind) {
        self.transport = transport
        hostKind = kind
    }

    /// The origin the core gets with a packet: the hub's attribution, or
    /// none over a link with no sessions.
    func origin(of note: LinkNotification) -> UInt16? {
        hostKind == .hub ? note.origin : nil
    }

    func kind() -> HostTransportKind {
        switch hostKind {
        case .fake: return .fake
        case .hub: return .link(session: transport.session)
        }
    }

    func resetBulkCache() {
        cacheLock.lock(); bulkCache = nil; cacheLock.unlock()
    }

    func controlIn(opcode: UInt8, value: UInt16, len: UInt16) throws -> Data {
        if hostKind == .fake && opcode == HostBridge.getAllParamsChunk {
            return try bulkChunk(offset: Int(value), len: Int(len))
        }
        switch transport.getControlResult(request: opcode, value: value, index: HostBridge.vendorInterface,
                                          length: len) {
        case .success(let data): return data
        case .failure(let status): throw HostBridge.error(for: status, opcode: opcode)
        }
    }

    /// `FakeTransport` has no chunked bulk read, and the core reads a fake
    /// link as it reads USB: chunks of one image, read at offset 0.
    private func bulkChunk(offset: Int, len: Int) throws -> Data {
        cacheLock.lock(); defer { cacheLock.unlock() }
        if offset == 0 || bulkCache == nil {
            switch transport.getControlResult(request: REQ_GET_ALL_PARAMS, value: 0, index: HostBridge.vendorInterface,
                                              length: UInt16(BULK_PARAMS_SIZE)) {
            case .success(let data): bulkCache = Data(data)
            case .failure(let status): throw HostBridge.error(for: status, opcode: HostBridge.getAllParamsChunk)
            }
        }
        guard let image = bulkCache, offset <= image.count else {
            throw DspiError.Refused(status: LinkStatus.error.rawValue)
        }
        return image.subdata(in: offset..<min(image.count, offset + len))
    }

    func controlOut(opcode: UInt8, value: UInt16, data: Data) throws {
        guard transport.isConnected else { throw DspiError.Disconnected }
        transport.sendControlRequest(request: opcode, value: value, index: HostBridge.vendorInterface, data: data)
    }

    func lock(reason: String, timeoutMs: UInt32) throws {
        guard transport.acquireDeviceLock(reason: reason, timeout: TimeInterval(timeoutMs) / 1000) else {
            throw DspiError.Locked
        }
    }

    func unlock() {
        transport.releaseDeviceLock()
    }

    /// A failed transfer as the error the core expects from a host
    /// (`HostTransport`'s doc): a stall is a refusal.
    static func error(for status: LinkStatus, opcode: UInt8) -> DspiError {
        switch status {
        case .noDevice: return .Disconnected
        case .locked: return .Locked
        case .denied: return .Denied
        case .unsupported: return .Unsupported
        case .error, .blocked: return .Refused(status: status.rawValue)
        default: return .Transfer(msg: "opcode 0x\(String(format: "%02X", opcode)): \(status)")
        }
    }
}
