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
    /// -- WP SW-M1 -- This is the tests' switch (CoreTransportUnderTest) and
    /// the iOS demo member's.  The Mac app uses the core by default
    /// (`AppState.usesCoreTransport`), where `-CoreTransport NO` opts out.
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
        /// -- WP P1SW1 -- A `.dspipreset` document opened offline (the core's
        /// `openDocument`, PRO1 W7): listed once `prepareDocument` names one,
        /// opened when it is selected.  Nothing touches USB.
        case document
        /// -- WP SW5 -- The DSPi's USB MIDI control port (CoreMIDI or the
        /// development relay): the core's MIDI transport over the pipe's
        /// I/O (MIDI/CoreMidiPipe.swift).  Lists the pipe's devices and
        /// follows its hot-plug.
        case midi(CoreMidiPipe)
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
    // -- WP SW8 -- What a drain changed in the core's mirror (CoreMirror.swift).
    private let stateFanout = CoreStateFanout()

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
    // -- WP SW5 -- The MIDI port this transport drives (`Link.midi`).
    let midi: CoreMidiPipe?
    /// -- WP SW5 -- Bumped by every MIDI open and close (stateLock): an
    /// open's outcome is reported only while it is still the latest.  Not
    /// `localGeneration`, which a drain also moves when the core changes
    /// its connection.
    private var midiOpens: UInt64 = 0
    private var subscriptions: Set<AnyCancellable> = []
    private var notifyListener: Listener?
    // -- WP P1SW1 --
    /// This transport carries documents opened offline (`Link.document`).
    let isDocumentLink: Bool
    /// The document to open when its device is selected (stateLock).
    private var document: (device: DSPiDevice, text: String, fileName: String)?
    private var documentCount = 0
    /// How the last document open went: the core's JSON reply, or its
    /// error.  Main thread.
    var documentOpened: ((Result<String, DspiError>) -> Void)?

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
    /// -- WP SWF6 -- Tests only: drains wait while set, so a notification
    /// window (a preset load's, say) stays open for as long as a test needs.
    private var drainHeldForTest = false
    /// -- WP SWF7 -- For `settlePresetChanges` (system uptime): when the last
    /// raw SET was handed to `coreQueue`, when it went to the core
    /// (coreQueue only), and when the core last reported a notification.
    private var lastRawSetAt: TimeInterval?
    private var lastRawSetSentAt: TimeInterval = 0
    private var lastNotifiedAt: TimeInterval = 0
    /// -- WP SWMAC1 -- The journal the last drain saw (coreQueue only).
    private var lastJournal = CoreJournalState()
    /// -- WP SW9 -- The write lane (SWIFT-SEAMS 5.2): the newest value
    /// queued per key, with the generation it was issued for (stateLock).
    private var laneWrites: [String: LaneWrite] = [:]
    /// -- WP SW9 -- fixup sw9 F2: the job that will send a key's queued
    /// value, by a token; a commit of the key retires it (stateLock).
    private var laneJobs: [String: UInt64] = [:]
    private var laneTokens: UInt64 = 0
    /// -- WP SW9 -- A publish of the state after typed writes is queued
    /// (coreQueue only).
    private var writePublishQueued = false

    private static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "CoreBackedTransport")
    #if DEBUG
    private let mainGetLock = NSLock()
    private var mainGets: Set<UInt8> = []
    /// -- WP SW11 -- Tests only: the opcode of every raw SET and GET the app
    /// sent through this transport (`DeviceTransport`'s calls), in order.
    private var rawOpcodes: [UInt8] = []
    #endif

    /// - Parameters:
    ///   - link: what carries the core's link to the device.
    ///   - autoSelect: open the demo device straight away (demo only; a host
    ///     link follows its transport's own selection).
    ///   - core: the core to drive; a new one by default.
    init(link: Link, autoSelect: Bool = false, core: DspiCore = DspiCore()) {
        self.core = core
        coreQueue.setSpecific(key: CoreBackedTransport.onCoreQueue, value: true)
        isDocumentLink = { () -> Bool in if case .document = link { return true }; return false }()   // -- WP P1SW1 --
        switch link {
        case .demo:
            host = nil
            bridge = nil
            midi = nil
            session = 1
            devicesSubject = CurrentValueSubject([FakeTransport.demoDevice])
        case .document:   // -- WP P1SW1 --
            host = nil
            bridge = nil
            midi = nil
            session = 1
            devicesSubject = CurrentValueSubject([])
        case .host(let transport, let kind):
            host = transport
            bridge = HostBridge(transport, kind: kind)
            midi = nil
            session = kind == .hub ? transport.session : 1
            devicesSubject = CurrentValueSubject([])
        case .midi(let pipe):   // -- WP SW5 -- the only host on the port, like USB: session 1
            host = nil
            bridge = nil
            midi = pipe
            session = 1
            devicesSubject = CurrentValueSubject(pipe.devices)
        }

        let listener = Listener { [weak self] in self?.scheduleDrain() }
        notifyListener = listener
        coreQueue.async { [core] in
            core.notifyTapSet(enabled: true)
            core.setNotifyListener(listener: listener)
            // -- WP P1SW2 -- Tokens required (P1TK): every gated call in this
            // app mints its token after the person confirmed (`*Confirmed`).
            // The core requires them from the start (P1TZ): no switch.
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
        } else if let pipe = midi {
            // -- WP SW5 -- The pipe's list and hot-plug (main thread).
            pipe.onDevicesChanged = { [weak self] devices in self?.midiDevicesChanged(devices) }
            pipe.onChange = { [weak self] change in self?.midiChanged(change) }
            if let reason = pipe.unavailableReason { errorSubject.send(reason) }
            if pipe.autoSelect, let first = pipe.devices.first {
                DispatchQueue.main.async { [weak self] in
                    if self?.selectedDevice == nil { self?.selectDevice(first) }
                }
            }
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
        if isDocumentLink { return selectDocument(device) }   // -- WP P1SW1 --
        if midi != nil { return openMidiPort(device, force: false) }   // -- WP SW5 --
        guard device == FakeTransport.demoDevice else { return }
        bumpGeneration()
        selectedSubject.send(device)
        coreQueue.async { [weak self] in self?.openDemo() }
    }

    func reconnect() {
        if let host = host { host.reconnect(); return }
        guard let device = selectedDevice else { return }
        publishConnected(false)
        if midi != nil { return openMidiPort(device, force: true) }   // -- WP SW5 --
        selectDevice(device)
    }

    func disconnect() {
        if let host = host { host.disconnect(); return }
        bumpGeneration()
        selectedSubject.send(nil)
        if let pipe = midi {   // -- WP SW5 -- a request in flight fails at once
            _ = nextMidiOpen()
            pipe.close()
        }
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

    // -- WP SW5 -- The DSPi's USB MIDI port (`Link.midi`).

    /// Open `device`'s MIDI endpoint and run the core's connect sequence over
    /// it.  Main thread.  Re-selecting the open device keeps the port and a
    /// probe in progress (reopening would drop a reply in flight).
    private func openMidiPort(_ device: DSPiDevice, force: Bool) {
        guard let pipe = midi else { return }
        switch pipe.open(device, force: force) {
        case .unknown:
            return
        case .alreadyOpen:
            selectedSubject.send(device)
            // The probe on it gave up: selecting it again is a retry.
            if !isConnected, errorMessage != nil { openMidiPort(device, force: true) }
        case .failed(let message):
            bumpGeneration()
            _ = nextMidiOpen()
            selectedSubject.send(device)
            errorSubject.send(message)
            publishConnected(false)
            coreQueue.async { [weak self] in self?.close() }
        case .opened(let connection):
            bumpGeneration()
            let open = nextMidiOpen()
            selectedSubject.send(device)
            errorSubject.send(nil)
            coreQueue.async { [weak self] in self?.openMidiOnCore(connection, pipe: pipe, open: open) }
        }
    }

    private func nextMidiOpen() -> UInt64 {
        stateLock.lock(); defer { stateLock.unlock() }
        midiOpens &+= 1
        return midiOpens
    }

    private func isLatestMidiOpen(_ open: UInt64) -> Bool {
        stateLock.lock(); defer { stateLock.unlock() }
        return midiOpens == open
    }

    // On coreQueue.  The probe proves the device answers before the
    // transport says connected: a port can exist on firmware that ignores
    // our SysEx.  A newer open or close (the generation moved) makes the
    // outcome stale: it is not reported.
    private func openMidiOnCore(_ connection: CoreMidiPipe.Connection, pipe: CoreMidiPipe, open: UInt64) {
        guard isLatestMidiOpen(open) else { return }
        var failure: Error?
        do {
            try onCore { try pipe.attach(connection, to: $0) }
        } catch {
            failure = error
        }
        guard isLatestMidiOpen(open) else { return }
        let connected = onCore { $0.isConnected() }
        let coreGeneration = onCore { $0.connectionGeneration() }
        stateLock.lock(); knownCoreGeneration = coreGeneration; stateLock.unlock()
        if let failure = failure {
            Self.log.error("DSPi MIDI port does not answer: \(String(describing: failure), privacy: .public)")
        }
        DispatchQueue.main.async {
            guard self.isLatestMidiOpen(open) else { return }
            self.errorSubject.send(failure.map { "The DSPi's MIDI port does not answer (\(CoreBackedTransport.describe($0)))." })
            self.publishConnected(connected)
        }
    }

    /// -- WP SW5F -- (review sw5 F2) The pipe's list.  A selected device
    /// that is no longer listed while nothing is open (its open was refused)
    /// is let go, so its return is offered again; an open one's departure
    /// comes as `.departed`.  Main thread.
    private func midiDevicesChanged(_ devices: [DSPiDevice]) {
        devicesSubject.send(devices)
        if let selected = selectedDevice, !devices.contains(selected), midi?.openDevice == nil {
            disconnect()
        }
    }

    /// The pipe's hot-plug.  Main thread.
    private func midiChanged(_ change: CoreMidiPipe.Change) {
        switch change {
        case .departed(let device):
            guard selectedDevice == device else { return }
            disconnect()
        case .replugged(let device):
            publishConnected(false)
            openMidiPort(device, force: true)
        case .appeared(let device):
            if selectedDevice == nil { selectDevice(device) }
        }
    }

    /// The app is back in the foreground (iOS).  iOS suspends a background
    /// app, so nothing was read meanwhile and the device may have dropped
    /// notifications for us.  -- WP SW5F -- (review sw5 F1) The core re-reads
    /// the device (`fetchAll`: it proves the device answers and refreshes the
    /// mirror the view model shows), the new state is published as a drain
    /// does, and the raw observers get a BULK_INVALIDATED; if the device does
    /// not answer, reconnect.  `fetchAll` reads the device unless it runs
    /// right after the connect's own read (`fresh_image`), which the view
    /// model's connect-time `readAll` has used up by then.  Main thread.
    func resumeMidi() {
        guard midi != nil, let device = selectedDevice else { return }
        guard isConnected else { reconnect(); return }
        stateLock.lock(); let open = midiOpens; stateLock.unlock()
        coreQueue.async { [weak self] in
            guard let self = self else { return }
            var answered = true
            do {
                try self.onCore { try $0.fetchAll() }
            } catch {
                answered = false
                Self.log.info("resume: fetchAll failed: \(String(describing: error), privacy: .public)")
            }
            if answered, self.isLatestMidiOpen(open), !self.stateFanout.isEmpty {
                // As `drain()` publishes a change of the core's mirror.
                let journal = self.journalOnCore()
                self.lastJournal = journal
                self.stateFanout.publish(CoreStateChange(flags: CoreStateChange.stateFlags,
                                                         state: self.onCore { $0.getState() },
                                                         presetChanges: self.onCore { $0.presetChanges() },
                                                         journal: journal))
            }
            DispatchQueue.main.async {
                guard self.isLatestMidiOpen(open), self.selectedDevice == device else { return }
                if answered {
                    Self.log.notice("Resumed: DSPi still answers on MIDI, re-reading state")
                    self.fanout.publish(LinkNotification(packet: Self.resyncPacket, origin: 0, receivedAt: Date()))
                } else {
                    Self.log.notice("Resumed: DSPi does not answer on MIDI, reconnecting")
                    self.reconnect()
                }
            }
        }
    }

    /// v2 BULK_INVALIDATED, source UNKNOWN: version, event, flags, seq,
    /// source, reserved x3 (HubTransport's resync).
    static let resyncPacket = Data([0x02, 0x03, 0, 0, 0, 0, 0, 0])

    // -- WP P1SW1 -- Documents opened offline.

    /// Name `text` (a `.dspipreset` document read from `fileName`) as this
    /// transport's one device, to be opened when it is selected (the view
    /// model's switch, after its unsaved-changes prompt).  Each document is
    /// a device of its own, so a second file is a switch too.  Main thread.
    func prepareDocument(text: String, fileName: String) -> DSPiDevice {
        stateLock.lock()
        documentCount += 1
        let device = DSPiDevice(serial: "FILE-\(documentCount)", locationID: 0, hub: nil,
                                remoteName: "File: \(fileName)")
        document = (device, text, fileName)
        stateLock.unlock()
        devicesSubject.send([device])
        return device
    }

    private func selectDocument(_ device: DSPiDevice) {
        stateLock.lock(); let doc = document; stateLock.unlock()
        guard let doc = doc, doc.device == device else { return }
        bumpGeneration()
        selectedSubject.send(device)
        coreQueue.async { [weak self] in self?.openDocumentOnCore(doc.text, fileName: doc.fileName) }
    }

    // On coreQueue.
    private func openDocumentOnCore(_ text: String, fileName: String) {
        let result: Result<String, DspiError>
        do {
            result = .success(try onCore { try $0.openDocument(text: text, fileName: fileName) })
            opened(error: nil)
        } catch {
            result = .failure(error as? DspiError ?? .Transfer(msg: String(describing: error)))
            opened(error: error)
        }
        DispatchQueue.main.async { self.documentOpened?(result) }
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
        lastNotifiedAt = ProcessInfo.processInfo.systemUptime   // -- WP SWF7 --
        stateLock.unlock()
        guard !already else { return }
        coreQueue.asyncAfter(deadline: .now() + .milliseconds(30)) { [weak self] in self?.drain() }
    }

    // On coreQueue.  Apply the queued packets to the core's mirror, hand the
    // tap's copies to the observers, and follow the connection.
    private func drain() {
        stateLock.lock()
        drainScheduled = false
        let held = drainHeldForTest
        stateLock.unlock()
        guard !held else { return }   // -- WP SWF6 --
        let result = onCore { $0.processNotifications() }
        let packets = onCore { $0.notifyTapDrain() }
        for raw in packets {
            // SWIFT-SEAMS 4.1: the echo filter's verdict is the origin.
            let origin: LinkSessionID = raw.own ? session : 0
            let at = Date(timeIntervalSince1970: TimeInterval(raw.receivedMs) / 1000)
            fanout.publish(LinkNotification(packet: raw.packet, origin: origin, receivedAt: at))
        }
        // -- WP SW8 -- The mirror after the change, after the tap's packets
        // (both reach main in this order).
        // -- WP SWMAC1 -- Also when only the journal moved (a typed write's
        // echo changes no value in the mirror).
        let journal = journalOnCore()
        let journalMoved = journal != lastJournal
        lastJournal = journal
        if result.flags & CoreStateChange.stateFlags != 0 || journalMoved, !stateFanout.isEmpty {
            // -- WP SWF6 -- With the core's unsaved-changes list after it.
            stateFanout.publish(CoreStateChange(flags: result.flags, state: onCore { $0.getState() },
                                                presetChanges: onCore { $0.presetChanges() },
                                                journal: journal))   // -- WP SWMAC1 --
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

    // -- WP SWF6 --
    /// Tests only: hold the notification drain (true), or let it run again
    /// and process what arrived meanwhile in one batch (false).
    func holdDrainForTest(_ hold: Bool) {
        stateLock.lock(); drainHeldForTest = hold; stateLock.unlock()
        if !hold { scheduleDrain() }
    }

    // MARK: - SET (host to device)

    /// The core picks the interface, so `index` is dropped.
    func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
        #if DEBUG
        noteRaw(request)   // -- WP SW11 --
        #endif
        stateLock.lock(); lastRawSetAt = ProcessInfo.processInfo.systemUptime; stateLock.unlock()   // -- WP SWF7 --
        coreQueue.async { [weak self] in
            guard let self = self else { return }
            self.lastRawSetSentAt = ProcessInfo.processInfo.systemUptime   // -- WP SWF7 --
            do {
                try self.onCore { try $0.rawSet(opcode: request, value: value, data: data) }
            } catch {
                self.writeRefused(error)
                Self.log.info("SET 0x\(String(format: "%02X", request), privacy: .public) wValue=\(value) failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    // On coreQueue.  A write the core or the hub refused.  `typed`: one of
    // the domains' typed setters (the write lane), which also puts back an
    // edit the core refused as Unsupported or that failed in transfer.
    private func writeRefused(_ error: Error, typed: Bool = false) {
        // -- WP SWF2 -- A write the hub refused is put back on screen
        // (hw-mac F3).  Read-only firmware also maps to .denied, but
        // that is not the hub's refusal: only the core's own two count.
        switch error as? DspiError {
        case .Denied?: RefusedWrite.handle(.denied, fanout: fanout)
        case .Locked?: RefusedWrite.handle(.locked, fanout: fanout)
        // -- WP SWF4 -- Read-only firmware: put the edit back too.
        case .ReadOnly?: RefusedWrite.handleReadOnly(fanout: fanout)
        // -- WP SW9 -- A typed setter refused the value before sending it:
        // say why and show the device's value again.
        case .InvalidArgument(let msg)?: RefusedWrite.handleInvalid(msg, fanout: fanout)
        // -- WP SW10 -- fixup sw10 F2: a feature this device's probe did not
        // find, or a transfer that failed: nothing reached the device (or
        // nothing is known to have), so the edit is put back to the core's
        // value at once and the status row says why.
        case .Unsupported? where typed: RefusedWrite.handleInvalid(RefusedWrite.unsupportedMessage, fanout: fanout)
        case .Transfer? where typed: RefusedWrite.handleInvalid(RefusedWrite.transferMessage, fanout: fanout)
        // -- WP P1SW2 -- The limits profile, the installer lock, a missing
        // token and the hub's named refusals: nothing was sent, so the edit
        // is put back and the status row shows the core's notice row.
        case let e? where CoreInstaller.isNamedRefusal(e):
            RefusedWrite.handleInvalid(CoreInstaller.line(for: e), fanout: fanout)
        default: break
        }
    }

    // MARK: - GET (device to host)

    func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, LinkStatus> {
        #if DEBUG
        if Thread.isMainThread { noteGetFromMain(request) }
        noteRaw(request)   // -- WP SW11 --
        #endif
        return onCoreSync {
            do {
                return .success(try self.onCore { try $0.rawGet(opcode: request, value: value, len: length) })
            } catch {
                let status = CoreBackedTransport.status(for: error)
                if status == .locked { DeviceLock.notice.post() }
                // -- WP SWF4 -- A write-as-read (preset load or save, commit)
                // on read-only firmware: what the caller showed goes back.
                if case .ReadOnly? = error as? DspiError { RefusedWrite.handleReadOnly(fanout: self.fanout) }
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

    // -- WP SW11 --
    private func noteRaw(_ request: UInt8) {
        mainGetLock.lock(); rawOpcodes.append(request); mainGetLock.unlock()
    }

    /// -- WP SW11 -- Tests only: the raw opcodes sent since the last call
    /// (a domain with a core attached sends none of its own).
    func takeRawOpcodesForTest() -> [UInt8] {
        mainGetLock.lock(); defer { rawOpcodes = []; mainGetLock.unlock() }
        return rawOpcodes
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

// MARK: - The core's state (WP SW8)

// -- WP SW8 -- The view model reads the device state from the core
// (docs/SWIFT-SEAMS.md 5.2).
extension CoreBackedTransport: CoreStateSource, CoreStateSourceProviding {
    var providedCoreStateSource: CoreStateSource? { self }

    func readAll() -> Result<DspState, DspiError> {
        onCoreSync {
            do {
                try self.onCore { try $0.fetchAll() }
                return .success(self.onCore { $0.getState() })
            } catch let error as DspiError {
                return .failure(error)
            } catch {
                return .failure(.Transfer(msg: String(describing: error)))
            }
        }
    }

    func readState() -> DspState {
        onCoreSync { self.onCore { $0.getState() } }
    }

    func addStateObserver(_ handler: @escaping (CoreStateChange) -> Void) -> AnyCancellable {
        stateFanout.add(handler)
    }

    // -- WP SWF6 -- The unsaved-changes baseline is the core's.

    func readPresetChanges() -> [String] {
        onCoreSync { self.onCore { $0.presetChanges() } }
    }

    func capturePresetBaseline() -> [String] {
        onCoreSync {
            // The echoes of the last raw SETs are in the mirror first.
            self.drain()
            self.onCore { $0.captureBaseline() }
            return self.onCore { $0.presetChanges() }
        }
    }

    // -- WP SWF7 --
    /// How recent a raw SET must be for a prompt to settle first: its echo
    /// arrives one drain (30 ms) plus the link's echo latency after it.  Also
    /// the longest the settle waits for the echoes after the last SET.
    static let settleWindow: TimeInterval = 0.1
    /// How long the link must be quiet before the settle drains.
    static let settleQuiet: TimeInterval = 0.02

    func settlePresetChanges() -> [String]? {
        // -- WP SWF8 -- Not over a hub (c72-N1): its echoes reach the core
        // through the main thread (HubTransport's fanout), which this call
        // holds, so waiting cannot bring them in.
        if bridge?.isHub == true { return nil }
        let start = ProcessInfo.processInfo.systemUptime
        stateLock.lock(); let at = lastRawSetAt; stateLock.unlock()
        guard let at = at, start - at < Self.settleWindow else { return nil }
        if DispatchQueue.getSpecific(key: CoreBackedTransport.onCoreQueue) == true {
            return settleOnCore(until: start + Self.settleWindow)
        }
        // -- WP SWF8 -- The whole call is bounded, the hop behind work already
        // on coreQueue included (c72-N2): past `settleWindow` the caller gets
        // nil (the list as delivered) and the late job does nothing.
        let done = DispatchSemaphore(value: 0)
        let box = SettleBox()
        coreQueue.async { [weak self] in
            if !box.abandoned, let self = self {
                box.result = self.settleOnCore(until: start + Self.settleWindow - Self.settleDrainReserve)
            }
            done.signal()
        }
        guard done.wait(timeout: .now() + Self.settleWindow) == .success else {
            box.abandoned = true
            return nil
        }
        return box.result
    }

    /// -- WP SWF8 -- Of the call's `settleWindow`, what the wait leaves for
    /// the drain.
    static let settleDrainReserve: TimeInterval = 0.02

    // On coreQueue.  Behind the SETs on the queue.  Their echoes reach the
    // core through the reader thread, one wake each: wait until the link has
    // been quiet for `settleQuiet` since the last SET or echo, at most the
    // window from the last SET (a write that echoes nothing waits only the
    // quiet time) and never past `limit`.
    private func settleOnCore(until limit: TimeInterval) -> [String] {
        let deadline = min(lastRawSetSentAt + Self.settleWindow, limit)
        while true {
            let now = ProcessInfo.processInfo.systemUptime
            stateLock.lock(); let heard = lastNotifiedAt; stateLock.unlock()
            if now >= deadline || now - max(heard, lastRawSetSentAt) >= Self.settleQuiet { break }
            Thread.sleep(forTimeInterval: 0.002)
        }
        drain()
        return onCore { $0.presetChanges() }
    }

    /// -- WP SWF8 -- The settle's result, and the caller giving up on it.
    private final class SettleBox: @unchecked Sendable {
        private let lock = NSLock()
        private var _result: [String]?
        private var _abandoned = false
        var result: [String]? {
            get { lock.lock(); defer { lock.unlock() }; return _result }
            set { lock.lock(); _result = newValue; lock.unlock() }
        }
        var abandoned: Bool {
            get { lock.lock(); defer { lock.unlock() }; return _abandoned }
            set { lock.lock(); _abandoned = newValue; lock.unlock() }
        }
    }

    // -- WP SWF8 -- c75-N3

    func readMaxDelayMs() -> Float? {
        onCoreSync { self.onCore { $0.maxDelayMs() } }
    }

    func outputDelayRefusal(output: UInt8, ms: Float, reply: @escaping (String?) -> Void) {
        coreQueue.async { [weak self] in
            var text: String?
            // Only past the line: there the typed setter refuses before it
            // sends anything (a negative it would send as 0).
            if let self = self, ms.isNaN || ms > self.onCore({ $0.maxDelayMs() }) {
                do {
                    try self.onCore { try $0.setOutputDelay(output: output, ms: ms) }
                } catch DspiError.InvalidArgument(let msg) {
                    text = msg
                } catch {}
            }
            DispatchQueue.main.async { reply(text) }
        }
    }

    // -- WP SWMAC1 -- The core's change journal.

    func readJournal() -> CoreJournalState {
        onCoreSync { self.journalOnCore() }
    }

    // On coreQueue.
    private func journalOnCore() -> CoreJournalState {
        CoreJournalState(canUndo: onCore { $0.canUndo() }, canRedo: onCore { $0.canRedo() })
    }

    func stepJournal(redo: Bool) -> CoreJournalStep {
        onCoreSync {
            // The echoes of the last raw SETs are in the mirror first.
            self.drain()
            let result: Result<HistoryMove, DspiError>
            do {
                result = .success(try self.onCore { redo ? try $0.redo() : try $0.undo() })
            } catch {
                let failure = error as? DspiError ?? .Transfer(msg: String(describing: error))
                // As a raw write's failure.
                switch failure {
                case .Locked: DeviceLock.notice.post()
                case .ReadOnly: RefusedWrite.handleReadOnly(fanout: self.fanout)
                default: break
                }
                result = .failure(failure)
            }
            let journal = self.journalOnCore()
            self.lastJournal = journal
            return CoreJournalStep(result: result, state: self.onCore { $0.getState() },
                                   presetChanges: self.onCore { $0.presetChanges() }, journal: journal)
        }
    }

    func setDragging(_ dragging: Bool) {
        coreQueue.async { [weak self] in self?.onCore { $0.setDragging(dragging: dragging) } }
    }

    // -- WP SW9 -- The domains' typed calls and the write lane (SWIFT-SEAMS 5.2).

    func write(lane key: String?, _ body: @escaping (DspiCore) throws -> Void) {
        stateLock.lock()
        lastRawSetAt = ProcessInfo.processInfo.systemUptime   // the unsaved-changes settle waits behind it
        let entry = LaneWrite(generation: localGeneration, body: body)
        guard let key = key else {
            stateLock.unlock()
            coreQueue.async { [weak self] in self?.runWrite(nil, entry) }
            return
        }
        laneWrites[key] = entry
        // The job already queued for this key will send the newest value.
        if laneJobs[key] != nil { stateLock.unlock(); return }
        laneTokens &+= 1
        let token = laneTokens
        laneJobs[key] = token
        stateLock.unlock()
        coreQueue.async { [weak self] in self?.runLane(key, token) }
    }

    // -- WP SW9 -- fixup sw9 F2
    func commit(lane key: String, _ body: @escaping (DspiCore) throws -> Void) {
        stateLock.lock()
        lastRawSetAt = ProcessInfo.processInfo.systemUptime
        let entry = LaneWrite(generation: localGeneration, body: body)
        // The commit supersedes the value still queued for its key, and the
        // job queued for it sends nothing now: a later value for the key gets
        // a job of its own behind this commit, so it cannot be overtaken.
        laneWrites[key] = nil
        laneJobs[key] = nil
        stateLock.unlock()
        coreQueue.async { [weak self] in self?.runWrite(key, entry) }
    }

    // On coreQueue.  The lane job of `key`: sends the newest value queued for
    // it, unless a commit retired this job.
    private func runLane(_ key: String, _ token: UInt64) {
        stateLock.lock()
        guard laneJobs[key] == token else { stateLock.unlock(); return }
        laneJobs[key] = nil
        let entry = laneWrites.removeValue(forKey: key)
        stateLock.unlock()
        guard let entry = entry else { return }
        runWrite(key, entry)
    }

    // On coreQueue.  A write issued for another connection is dropped, and a
    // failed one drops every queued lane value: nothing is replayed.
    private func runWrite(_ key: String?, _ entry: LaneWrite) {
        stateLock.lock()
        let current = localGeneration
        stateLock.unlock()
        guard entry.generation == current else { return }
        lastRawSetSentAt = ProcessInfo.processInfo.systemUptime
        do {
            try onCore { try entry.body($0) }
        } catch {
            stateLock.lock(); laneWrites.removeAll(); laneJobs.removeAll(); stateLock.unlock()
            writeRefused(error, typed: true)
            Self.log.info("typed write \(key ?? "", privacy: .public) failed: \(String(describing: error), privacy: .public)")
            return
        }
        guard !writePublishQueued else { return }
        writePublishQueued = true
        coreQueue.async { [weak self] in self?.publishAfterWrites() }
    }

    // On coreQueue, behind the writes queued with it.  A typed write patched
    // the core's mirror and its journal at once, and its echo may change
    // nothing there, so no drain would say so: publish the state after the
    // writes, with the unsaved-changes list and the journal (Undo enabled).
    // Once per run of queued writes, so the last state the view model holds
    // is the one after the last write (a pending field's sweep applies it).
    private func publishAfterWrites() {
        writePublishQueued = false
        let journal = journalOnCore()
        lastJournal = journal
        guard !stateFanout.isEmpty else { return }
        stateFanout.publish(CoreStateChange(flags: 0, state: onCore { $0.getState() },
                                            presetChanges: onCore { $0.presetChanges() }, journal: journal,
                                            confirms: false))
    }

    // -- WP SW11 -- A typed write whose reply the caller needs (a pin
    // setter's status), then the state publish a queued write gets.
    func writeAndWait<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> {
        stateLock.lock(); lastRawSetAt = ProcessInfo.processInfo.systemUptime; stateLock.unlock()
        return onCoreSync {
            self.lastRawSetSentAt = ProcessInfo.processInfo.systemUptime
            let result: Result<T, DspiError>
            do {
                result = .success(try self.onCore(body))
            } catch {
                let failure = error as? DspiError ?? .Transfer(msg: String(describing: error))
                // -- WP SW12 -- review sw11 N1: as `runWrite`, a failed write
                // drops every queued lane value and is put back with the
                // core's wording (fixup sw10 F2's cases included).
                self.stateLock.lock(); self.laneWrites.removeAll(); self.laneJobs.removeAll(); self.stateLock.unlock()
                self.writeRefused(failure, typed: true)
                result = .failure(failure)
            }
            self.queuePublishAfterWrites()
            return result
        }
    }

    // -- WP SW12 -- review sw11 N2: a typed re-read (no write), behind the
    // writes queued before it, then the state published as after a write. It
    // stamps no write time (the unsaved-changes settle does not wait for it)
    // and posts no refusal.
    func readAndPublish<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> {
        onCoreSync {
            let result: Result<T, DspiError>
            do {
                result = .success(try self.onCore(body))
            } catch {
                result = .failure(error as? DspiError ?? .Transfer(msg: String(describing: error)))
            }
            self.queuePublishAfterWrites()
            return result
        }
    }

    // -- WP SW12 -- fixup sw12 F2: as `writeRefused` shows a refusal.
    func postRefusal(_ message: String, resync: Bool) {
        if resync {
            RefusedWrite.handleInvalid(message, fanout: fanout)
        } else {
            DeviceLock.notice.post(message)
        }
    }

    // -- WP SW14 --
    func perform(_ body: @escaping (DspiCore) -> Void) {
        coreQueue.async { [weak self] in self?.onCore(body) }
    }

    // -- WP SW12 -- On coreQueue: one state publish behind the work queued now.
    private func queuePublishAfterWrites() {
        guard !writePublishQueued else { return }
        writePublishQueued = true
        coreQueue.async { [weak self] in self?.publishAfterWrites() }
    }

    func call<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> {
        onCoreSync {
            do {
                return .success(try self.onCore(body))
            } catch {
                let failure = error as? DspiError ?? .Transfer(msg: String(describing: error))
                switch failure {
                case .Locked: DeviceLock.notice.post()
                case .ReadOnly: RefusedWrite.handleReadOnly(fanout: self.fanout)
                // -- WP SW9 -- fixup sw9 N1: a hub's refusal, as for a write.
                case .Denied: RefusedWrite.handle(.denied, fanout: self.fanout)
                default: break
                }
                return .failure(failure)
            }
        }
    }

    func loadPreset(slot: UInt8) -> Result<(DspState, [String]), DspiError> {
        onCoreSync {
            do {
                try self.onCore { try $0.loadPreset(slot: slot) }
                return .success((self.onCore { $0.getState() }, self.onCore { $0.presetChanges() }))
            } catch {
                let failure = error as? DspiError ?? .Transfer(msg: String(describing: error))
                // As a raw write-as-read's failure (getControlResult).
                switch failure {
                case .Locked: DeviceLock.notice.post()
                // -- WP P1SW2 -- the lock or the limits refused it: the core's row
                case let e where CoreInstaller.isNamedRefusal(e): DeviceLock.notice.post(CoreInstaller.line(for: e))
                case .ReadOnly: RefusedWrite.handleReadOnly(fanout: self.fanout)
                default: break
                }
                return .failure(failure)
            }
        }
    }
}

// -- WP SW9 --
/// One typed write as issued: the connection it was meant for.
private struct LaneWrite {
    let generation: UInt64
    let body: (DspiCore) throws -> Void
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
    private static let setAllParamsChunk: UInt8 = 0xA3   // -- WP SW13 --
    private let cacheLock = NSLock()
    private var bulkCache: Data?
    /// -- WP SW13 -- The chunks of a bulk write to a fake, until the image is whole.
    private var bulkWrite: Data?

    init(_ transport: any DeviceTransport, kind: CoreBackedTransport.HostKind) {
        self.transport = transport
        hostKind = kind
    }

    /// The origin the core gets with a packet: the hub's attribution, or
    /// none over a link with no sessions.
    func origin(of note: LinkNotification) -> UInt16? {
        hostKind == .hub ? note.origin : nil
    }

    /// -- WP SWF8 -- A hub session: its notifications arrive on main.
    var isHub: Bool { hostKind == .hub }

    func kind() -> HostTransportKind {
        switch hostKind {
        case .fake: return .fake
        case .hub: return .link(session: transport.session)
        }
    }

    func resetBulkCache() {
        cacheLock.lock(); bulkCache = nil; bulkWrite = nil; cacheLock.unlock()
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
            // -- WP P1SW3 -- as much as wLength carries, as the core asks a
            // single-shot read (the firmware answers min(wLength, image)): a
            // newer firmware's image (the V33 fixture) is longer than V32's.
            switch transport.getControlResult(request: REQ_GET_ALL_PARAMS, value: 0, index: HostBridge.vendorInterface,
                                              length: UInt16.max) {
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
        if hostKind == .fake && opcode == HostBridge.setAllParamsChunk {   // -- WP SW13 --
            return try bulkWriteChunk(offset: Int(value), data: data)
        }
        transport.sendControlRequest(request: opcode, value: value, index: HostBridge.vendorInterface, data: data)
    }

    /// -- WP SW13 -- `FakeTransport` takes a bulk image only whole, as one
    /// `REQ_SET_ALL_PARAMS`, and the core writes a fake link as it writes
    /// USB, in chunks: gather them in order, and send the image once it is
    /// whole (the firmware applies it once its last byte lands).
    private func bulkWriteChunk(offset: Int, data: Data) throws {
        cacheLock.lock()
        if offset == 0 { bulkWrite = Data() }
        guard var image = bulkWrite, image.count == offset else {
            bulkWrite = nil
            cacheLock.unlock()
            throw DspiError.Refused(status: LinkStatus.error.rawValue)
        }
        image.append(data)
        let whole = image.count >= Int(BULK_PARAMS_SIZE)
        bulkWrite = whole ? nil : image
        if whole { bulkCache = nil }
        cacheLock.unlock()
        if whole {
            transport.sendControlRequest(request: REQ_SET_ALL_PARAMS, value: 0, index: HostBridge.vendorInterface,
                                         data: image)
        }
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
