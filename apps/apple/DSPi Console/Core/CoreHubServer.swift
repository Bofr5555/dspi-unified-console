//
//  CoreHubServer.swift
//  DSPi Console
//
//  -- WP P2SW -- The hub's own families for the Swift screens (PLAN-P2: the
//  shared snapshots, the hub's history, alarm notices, Identify), as Qt's
//  HubFeed and Android's HubServer (P2QT, P2AN).  Every rule is the core's
//  (hub_server.rs, P2CC1/P2CC2): which families the hub has (`hubCaps`), the
//  merge of both sides' snapshots with the hub's winning, an alarm's notice
//  row, Identify through the hub or the device's own EXT_IDENTIFY.  This
//  carries the calls off main, keeps the caps and the alarms observable and
//  hands the hub's events (`event.added`, `snapshot.changed`,
//  `alarm.changed`) to main.  A hub without a family shows nothing of it:
//  every call answers `unsupported` there and sends nothing.
//
//  Main thread, but the calls, which run on `HubServerModel.work`.
//

import Foundation
import Combine
import dspi_core

/// The hub's families of the open device.  Main thread.
final class HubServerModel: ObservableObject {
    static let shared = HubServerModel(vm: AppState.shared.viewModel, notices: .main)

    /// One page of history.
    static let eventsPage: UInt32 = 200
    /// How long Identify blinks, seconds.
    static let identifySeconds: UInt8 = 10

    /// `MergedSnapshot.location`.
    static let whereLocal = "local", whereHub = "hub", whereBoth = "both"
    /// `HubAlarm.state`.
    static let alarmRaised = "raised", alarmCleared = "cleared"
    /// `HubServerEvent.kind`.
    static let eventAdded = "event.added", snapshotChanged = "snapshot.changed", alarmChanged = "alarm.changed"

    static let noCaps = HubCaps(onHub: false, inventory: false, identify: false, health: false, events: false,
                                hubSnapshots: false, paramApi: false, hubStats: false, alarms: false,
                                outbound: false, schedule: false, mqtt: false, apiKeys: false, rest: false,
                                // -- review/swsec -- wave P3-C's fleet caps (main 462fcde1)
                                groups: false, groupParams: false, groupApply: false, rollout: false,
                                // -- WP P5HUB -- the hub's measurement documents
                                hubMeasure: false,
                                caps: [])

    /// The hub's families (`hubCaps`, no I/O), read again on every device change.
    @Published private(set) var caps = HubServerModel.noCaps
    /// The hub's alarms that are on and concern the open device or no one
    /// device, newest first.  Empty without the `alarms` cap.
    @Published private(set) var alarms: [HubAlarm] = []
    /// Bumped by `snapshot.changed` for the open device: the Snapshots window reloads.
    @Published private(set) var snapshotsGeneration = 0
    /// The open device's history records added on the hub while watched.
    let eventAdded = PassthroughSubject<HubEventRecord, Never>()

    /// The queue the calls wait on the core from.
    static let work = DispatchQueue(label: "se.bofr.dspi.hubserver", qos: .userInitiated)

    private weak var vm: DSPViewModel?
    private let notices: NoticeCenter
    private var cancellables = Set<AnyCancellable>()
    /// Alarm ids already announced (main).
    private var announced = Set<String>()
    /// The cores the listener and the event folder were given to.
    private var wired = Set<ObjectIdentifier>()
    private var watching = false
    /// The local event log's folder; nil keeps it in memory (tests).
    let eventFolder: URL?

    init(vm: DSPViewModel, notices: NoticeCenter,
         eventFolder: URL? = CoreSnapshots.appFolder("events")) {
        self.vm = vm
        self.notices = notices
        self.eventFolder = eventFolder
        notices.acknowledge = { [weak self] id in self?.acknowledge(id) }
        vm.$isDeviceConnected.removeDuplicates().receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.deviceChanged() }.store(in: &cancellables)
        vm.$selectedDevice.map { $0?.serial }.removeDuplicates().receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.deviceChanged() }.store(in: &cancellables)
    }

    private var source: CoreStateSource? { vm?.transport.coreStateSource }
    private var serial: String { vm?.selectedDevice?.serial ?? "" }
    var connected: Bool { (vm?.isDeviceConnected ?? false) && source != nil }

    /// Identify can be offered: a device is open and, through a hub, the hub has the cap.
    var identifyOffered: Bool { connected && (!caps.onHub || caps.identify) }

    // MARK: Pure rules (tests)

    /// The alarms a single-device console shows: those of `serial` and those
    /// about no one device; cleared ones dropped, in the hub's order.
    static func relevant(_ all: [HubAlarm], serial: String) -> [HubAlarm] {
        all.filter { $0.state != alarmCleared && ($0.serial.isEmpty || $0.serial == serial) }
    }

    /// `now` with `changed` applied (`alarm.changed`): a cleared one leaves,
    /// others are put first or replaced.
    static func applyChange(_ now: [HubAlarm], _ changed: HubAlarm, serial: String) -> [HubAlarm] {
        let rest = now.filter { $0.id != changed.id }
        return relevant([changed], serial: serial).isEmpty ? rest : [changed] + rest
    }

    /// An alarm's notice: the core's row (`hubAlarmNotice`) with its id, so the
    /// host offers Acknowledge.
    static func notice(for alarm: HubAlarm) -> ConsoleNotice {
        let n = ConsoleNotice(hubAlarmNotice(alarm: alarm, time: ""))
        return ConsoleNotice(key: n.key, kind: n.kind, title: n.title, reason: n.reason, action: n.action,
                             detail: n.detail, alarmID: alarm.id)
    }

    // MARK: Calls

    /// `body` on the work queue, off main; `done` on main.
    private func run<T>(_ body: @escaping (DspiCore) throws -> T,
                        done: @escaping (Result<T, DspiError>) -> Void = { _ in }) {
        guard let source = source else {
            done(.failure(.Disconnected))
            return
        }
        Self.work.async {
            let result = source.call(body)
            DispatchQueue.main.async { done(result) }
        }
    }

    /// A failure in the user's words (the core's row for a refusal).
    static func words(_ error: DspiError) -> String {
        if CoreInstaller.isNamedRefusal(error) { return CoreInstaller.line(for: error) }
        return CoreSnapshots.words(error)
    }

    private func deviceChanged() {
        guard connected, let source = source else {
            caps = Self.noCaps
            if !alarms.isEmpty { alarms = [] }
            return
        }
        let key = ObjectIdentifier(source)
        if !wired.contains(key) {
            wired.insert(key)
            let listener = Listener(self)
            let dir = eventFolder?.path
            run({ core in
                core.setHubServerListener(listener: listener)
                if let dir { _ = try? core.eventLogSetDir(dir: dir) }
            })
        }
        run({ core in core.hubCaps() }) { [weak self] result in
            guard let self = self else { return }
            self.caps = (try? result.get()) ?? Self.noCaps
            self.announced.removeAll()
            self.refreshAlarms()
            if self.watching { self.watchEvents(true) }
        }
    }

    /// The hub's events, on a core thread: posted to main.
    private final class Listener: HubServerListener, @unchecked Sendable {
        private weak var model: HubServerModel?
        init(_ model: HubServerModel) { self.model = model }
        func onHubServerEvent(event: HubServerEvent) {
            DispatchQueue.main.async { [weak self] in self?.model?.handle(event) }
        }
    }

    /// One of the hub's events.  Main thread.
    func handle(_ e: HubServerEvent) {
        let serial = self.serial
        let mine = { (s: String) in s.isEmpty || serial.isEmpty || s == serial }
        switch e.kind {
        case Self.eventAdded:
            if let r = e.record, mine(r.serial) { eventAdded.send(r) }
        case Self.snapshotChanged:
            if mine(e.serial) { snapshotsGeneration &+= 1 }
        case Self.alarmChanged:
            if let a = e.alarm {
                alarms = Self.applyChange(alarms, a, serial: serial)
                announce([a])
            }
        default:
            break
        }
    }

    /// A notice for each alarm on that was not announced yet, oldest first.
    private func announce(_ list: [HubAlarm]) {
        for a in Self.relevant(list, serial: serial).reversed()
        where a.state == Self.alarmRaised && announced.insert(a.id).inserted {
            notices.post(Self.notice(for: a))
        }
    }

    /// Read the alarms again (`alarm.list`); nothing without the cap.
    func refreshAlarms() {
        guard caps.alarms else {
            if !alarms.isEmpty { alarms = [] }
            return
        }
        run({ core in try core.hubAlarms(serial: "", state: "active") }) { [weak self] result in
            guard let self = self, case .success(let all) = result else { return }
            self.alarms = Self.relevant(all, serial: self.serial)
            self.announce(all)
        }
    }

    /// Acknowledge alarm `id` (Control or Admin); a refusal is a notice.
    func acknowledge(_ id: String) {
        run({ core in try core.hubAlarmAck(id: id) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let a): self.alarms = Self.applyChange(self.alarms, a, serial: self.serial)
            case .failure(let e): self.notices.post(.of(e, detail: "The alarm was not acknowledged."))
            }
        }
    }

    /// Identify the open device (`hubIdentify`): through a hub with the cap
    /// the hub sends it, connected directly the device's own EXT_IDENTIFY.
    /// `seconds` 0 stops the blink.  The outcome is a notice.
    func identify(seconds: UInt8 = HubServerModel.identifySeconds,
                  done: @escaping (Result<IdentifyInfo?, DspiError>) -> Void = { _ in }) {
        run({ core in try core.hubIdentify(seconds: seconds, toneOutput: nil) }) { [weak self] result in
            switch result {
            case .success:
                self?.notices.post(result: seconds == 0 ? "Identify stopped."
                                                        : "The device blinks for \(seconds) s.")
            case .failure(let e):
                self?.notices.post(.of(e, detail: "Identify did not start."))
            }
            done(result)
        }
    }

    /// This console's own event log of the open device, newest first.
    func localEvents(done: @escaping ([EventEntry]) -> Void) {
        let limit = Self.eventsPage
        run({ core in core.eventLogRead(serial: "", afterId: 0, limit: limit).entries }) { result in
            done(((try? result.get()) ?? []).reversed())
        }
    }

    /// The open device's history on the hub (`event.list`), newest page.
    func hubEvents(done: @escaping (Result<HubEventPage, DspiError>) -> Void) {
        let limit = Self.eventsPage
        run({ core in try core.hubEventsRead(serial: "", afterId: 0, limit: limit) }, done: done)
    }

    /// Have the hub send the open device's new records while `on`. Best effort.
    func watchEvents(_ on: Bool) {
        watching = on
        guard caps.events else { return }
        let serial = self.serial
        run({ core in try core.hubEventsWatch(on: on, serial: serial) })
    }

    // MARK: Shared snapshots

    /// Both sides' snapshots of the open device (`snapshotListMerged`).
    func merged(done: @escaping (SnapshotMergedList) -> Void) {
        run({ core in core.snapshotListMerged(serial: "") }) { result in
            done((try? result.get()) ?? SnapshotMergedList(entries: [], hubListed: false, hubNoticeKey: ""))
        }
    }

    func saveToHub(_ id: String, done: @escaping (Result<HubSnapshotPut, DspiError>) -> Void) {
        run({ core in try core.hubSnapshotsSave(id: id) }, done: done)
    }

    func copyHere(_ id: String, done: @escaping (Result<SnapshotInfo, DspiError>) -> Void) {
        run({ core in try core.hubSnapshotsCopyHere(id: id) }, done: done)
    }

    func renameOnHub(_ id: String, _ name: String, done: @escaping (Result<SnapshotInfo, DspiError>) -> Void) {
        run({ core in try core.hubSnapshotsRename(id: id, name: name) }, done: done)
    }

    func deleteFromHub(_ id: String, done: @escaping (Result<Void, DspiError>) -> Void) {
        run({ core in try core.hubSnapshotsDelete(id: id) }, done: done)
    }
}
