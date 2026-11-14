//
//  CoreSnapshots.swift
//  DSPi Console
//
//  WP P1SW1.  Snapshots, the A/B compare and the safe apply (PLAN-CLIENTS
//  5.9: P1SN, P1AB, P1SA) through the Rust core, as the Qt app's
//  `SnapshotsController` and Android's `SnapshotsModel` (P1QT1, P1AN1,
//  P1DCFIX).  The core keeps the snapshots (one folder per serial under the
//  app's Application Support folder), runs the compare and the guard; this
//  holds the list, the user's choices and the one-line results.  A success
//  is the change itself or a notice line, never a modal.
//
//  - A recall is guarded when the core says so by default (DSPi Link to a
//    hub with the guard: `applyGuardDefault`, owner OD-P1g, fixup p1cfix) or
//    when the user asked for it on USB ("Recall with rollback").
//  - The A/B toggle is never guarded, and is off while a guard waits; a guard
//    that ends other than kept ends the compare (the core's rule), so the
//    compare is read again afterwards.
//  - Every call runs off the main thread, on the core's queue
//    (`CoreStateSource.call`); a recall, a toggle and a guarded apply end
//    the undo history, so the view model's state and the journal are read
//    again after each.
//  - Only while the device is on the core (`-CoreTransport YES`, the core's
//    demo device, or a file opened offline); otherwise `available` is false.
//

import Foundation
import Combine
import SwiftUI
import dspi_core
import dspi_constants
#if os(macOS)
import AppKit
#endif

// MARK: - P1DF's diff

/// One changed value: the import block that carries it, where it is, and
/// the value before and after.
struct SnapshotDiffRow: Identifiable, Equatable {
    let id: Int
    let block: String
    let section: String
    let item: String
    let before: String
    let after: String

    /// "Section · item".
    var place: String { [section, item].filter { !$0.isEmpty }.joined(separator: " · ") }
}

/// P1DF's diff JSON (`config_diff`, `config_preview`, `ab_diff`):
/// `{ok, error, rows, count, blocks, summary, notes, kind}`.
struct SnapshotDiff: Equatable {
    var ok: Bool
    var error = ""
    var rows: [SnapshotDiffRow] = []
    var summary: [String] = []
    var notes: [String] = []

    /// Rows per block key.
    var countByBlock: [String: Int] {
        rows.reduce(into: [:]) { $0[$1.block, default: 0] += 1 }
    }

    static func parse(_ json: String) -> SnapshotDiff {
        guard let object = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] else {
            return SnapshotDiff(ok: false, error: "The changes could not be read.")
        }
        guard object["ok"] as? Bool == true else {
            let error = (object["error"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            return SnapshotDiff(ok: false, error: error ?? "The changes could not be listed.")
        }
        let rows = (object["rows"] as? [[String: Any]] ?? []).enumerated().map { index, row in
            SnapshotDiffRow(id: index,
                            block: row["block"] as? String ?? "",
                            section: row["section"] as? String ?? "",
                            item: row["item"] as? String ?? "",
                            before: text(row["before"]),
                            after: text(row["after"]))
        }
        return SnapshotDiff(ok: true, rows: rows,
                            summary: object["summary"] as? [String] ?? [],
                            notes: object["notes"] as? [String] ?? [])
    }

    /// A JSON value as the row shows it ("" for none).
    static func text(_ value: Any?) -> String {
        switch value {
        case nil, is NSNull: return ""
        case let s as String: return s
        case let n as NSNumber:
            if CFGetTypeID(n) == CFBooleanGetTypeID() { return n.boolValue ? "on" : "off" }
            let d = n.doubleValue
            if d == d.rounded(), abs(d) < 1e15 { return String(Int64(d)) }
            return String(d)
        default: return String(describing: value!)
        }
    }
}

/// The blocks an import can apply alone (PRO1 W8), in the core's order.
/// `key` is the diff rows' `block`.
enum ImportBlock: String, CaseIterable, Identifiable {
    case eq, crossover, routing, outputs, limiters, effects, volumes, hardware, names

    var id: String { rawValue }
    var key: String { rawValue }

    var title: String {
        switch self {
        case .eq: return "EQ"
        case .crossover: return "Crossover"
        case .routing: return "Routing and matrix"
        case .outputs: return "Outputs"
        case .limiters: return "Limiters"
        case .effects: return "Effects"
        case .volumes: return "Volume levels"
        case .hardware: return "Hardware I/O"
        case .names: return "Names"
        }
    }

    var bit: UInt32 {
        switch self {
        case .eq: return DspiConstants.IMPORT_EQ
        case .crossover: return DspiConstants.IMPORT_CROSSOVER
        case .routing: return DspiConstants.IMPORT_ROUTING
        case .outputs: return DspiConstants.IMPORT_OUTPUTS
        case .limiters: return DspiConstants.IMPORT_LIMITERS
        case .effects: return DspiConstants.IMPORT_EFFECTS
        case .volumes: return DspiConstants.IMPORT_VOLUMES
        case .hardware: return DspiConstants.IMPORT_HARDWARE
        case .names: return DspiConstants.IMPORT_NAMES
        }
    }

    /// Every block's bit: the preview lists every change.
    static var allBits: UInt32 { allCases.reduce(0) { $0 | $1.bit } }

    /// `options` for the core's selected import of `blocks`.
    static func options(_ blocks: Set<ImportBlock>) -> UInt32 {
        DspiConstants.IMPORT_BY_BLOCK | blocks.reduce(0) { $0 | $1.bit }
    }

    /// What is ticked at first: every block with a change, except the volume
    /// levels and the hardware I/O, which an import left alone unless asked
    /// (as before, and as Qt and Android).
    static func defaults(_ diff: SnapshotDiff) -> Set<ImportBlock> {
        let changed = Set(diff.countByBlock.keys.compactMap(ImportBlock.init(rawValue:)))
        let base = diff.ok ? changed : Set(allCases)
        return base.subtracting([.volumes, .hardware])
    }
}

/// Every wire channel's EQ (`eq_channels` = `u32::MAX`).
let allEqChannels: UInt32 = .max

// MARK: - The wrapper

/// A notice line (no modal): what an action did, or why not.
struct SnapshotsNotice: Equatable {
    enum Kind { case info, warning, error }
    let text: String
    let kind: Kind
}

/// The snapshots, the compare and the guard of one view model.  Main thread.
final class CoreSnapshots: ObservableObject {
    static let shared = CoreSnapshots(vm: AppState.shared.viewModel)

    // -- The store --
    /// The open device's snapshots, newest first.
    @Published private(set) var snapshots: [SnapshotInfo] = []
    /// A call of this wrapper is under way: the buttons wait.
    @Published private(set) var busy = false
    /// The device is on the core: snapshots work here.
    @Published private(set) var available = false
    /// A recall or an import is guarded without asking (DSPi Link to a hub
    /// with the guard).
    @Published private(set) var guardDefault = false
    /// The user's choice for a recall on USB: with rollback (Keep or Revert).
    @Published var rollback = false
    /// The last action's outcome in one line, nil none.
    @Published private(set) var notice: SnapshotsNotice?

    // -- A/B --
    @Published private(set) var compare = CoreSnapshots.noCompare
    /// What differs from A to B while a compare runs.
    @Published private(set) var abRows: [SnapshotDiffRow] = []
    @Published private var toggling = 0

    // -- The safe apply --
    @Published private(set) var guardState = CoreSnapshots.idleGuard
    /// The countdown's sentence (the core's notice `guard.waiting`; `{s}` the
    /// whole seconds left).
    @Published private(set) var countdownWords = CoreSnapshots.fallbackCountdown
    /// How long a guarded apply waits for Keep, seconds: 30 unless set
    /// (Advanced), 5 to 300, remembered on this console.
    @Published var timeoutS: Int {
        didSet {
            let s = Self.clampTimeout(timeoutS)
            if s != timeoutS { timeoutS = s }   // no second didSet from inside one
            UserDefaults.standard.set(s, forKey: Self.timeoutKey)
        }
    }
    /// The id of the guarded apply this console saw waiting (its end is shown).
    @Published private(set) var watchedGuard: UInt64 = 0
    /// The end the user dismissed (id), so the bar hides.
    @Published private(set) var dismissedEnd: UInt64?

    // -- Offline files --
    /// The open document, when the connection is a file (`offlineDocument`).
    @Published private(set) var offline: OfflineDocumentInfo?

    /// Held strongly: the wrapper keeps its view model (the app's lives as long as the app).
    private var vm: DSPViewModel?
    private var running = 0
    private var cancellables = Set<AnyCancellable>()
    private var pollTimer: Timer?
    private var polling = false
    private var noticeClear: DispatchWorkItem?
    /// The cores whose store folder was named (by identity).
    private var folderSet = Set<ObjectIdentifier>()
    /// The store's folder; nil names none (tests that never write).
    let folder: URL?
    #if os(macOS)
    private var keyMonitor: Any?
    #endif

    /// The queue the calls wait on the core from: serial, in the order the
    /// user made them, and a thread of its own (a private queue gets one even
    /// when the shared pool is busy, where `DispatchQueue.global` waits).
    static let work = DispatchQueue(label: "com.dspi.snapshots", qos: .userInitiated)

    static let timeoutKey = "snapshots.guardTimeoutS"
    static let timeoutDefaultS = 30
    static let timeoutRange = 5...300
    static let nameMax = 200
    static let noteMax = 4000
    /// The poll's tick while a guard waits (the countdown's resolution).
    static let pollWaiting: TimeInterval = 0.25
    /// While the connection is lost: ask how it ended.
    static let pollLost: TimeInterval = 1.0

    /// The core's refusal while a guard waits, in the plan's words (P1CFIX).
    static let finishFirst = "Finish the safe apply first: Keep or Revert."
    /// The core's `guard.waiting` title and reason, for a core without them.
    static let fallbackCountdown = "Keep these settings? They go back in {s} s unless you press Keep."
    static let countdownKey = "guard.waiting"
    /// P1WB2's row (not on main when this was written): its key and the
    /// wording until it is.
    static let hubBackstopKey = "guard.hub_backstop"
    static let fallbackHubBackstop = "The hub puts them back too if this console goes away."
    /// "Apply with rollback": what it does, in one line.
    static let rollbackExplained = "After the import, Keep or Revert. Without an answer the settings from before come back."

    static let noCompare = AbCompare(active: false, side: nil, aName: "", bName: "", recall: nil)
    static let idleGuard = GuardedApply(id: 0, phase: .idle, label: "", hub: false, timeoutMs: 0, remainingMs: 0,
                                        reason: "", message: "", changes: [], notes: [])

    static func clampTimeout(_ s: Int) -> Int { min(max(s, timeoutRange.lowerBound), timeoutRange.upperBound) }

    /// The app's own folder for `name` ("snapshots", "templates").
    static func appFolder(_ name: String) -> URL? {
        guard let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
            return nil
        }
        return base.appendingPathComponent(AppIdentity.supportFolder(legacy: "com.weeblabs.DSPi-Console"), isDirectory: true)
            .appendingPathComponent(name, isDirectory: true)
    }

    /// - Parameters:
    ///   - vm: the view model whose transport carries the core.
    ///   - folder: the store's folder (the app's own by default).
    init(vm: DSPViewModel, folder: URL? = CoreSnapshots.appFolder("snapshots")) {
        self.vm = vm
        self.folder = folder
        let stored = UserDefaults.standard.integer(forKey: Self.timeoutKey)
        timeoutS = Self.clampTimeout(stored == 0 ? Self.timeoutDefaultS : stored)
        countdownWords = Self.wordsFor(key: Self.countdownKey, fallback: Self.fallbackCountdown, needs: "{s}")
        vm.$isDeviceConnected
            .removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] connected in self?.connectionChanged(connected) }
            .store(in: &cancellables)
        vm.$selectedDevice
            .removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.connectionChanged(self?.vm?.isDeviceConnected ?? false) }
            .store(in: &cancellables)
    }

    deinit {
        pollTimer?.invalidate()
        #if os(macOS)
        if let monitor = keyMonitor { NSEvent.removeMonitor(monitor) }
        #endif
    }

    /// A notice row's title and reason ("" parts left out), or `fallback`
    /// when the table has no row for `key` (or it lacks `needs`).  The table
    /// only: no handle, no I/O.
    static func wordsFor(key: String, fallback: String, needs: String = "") -> String {
        let n = noticeForKey(key: key, detail: "")
        guard n.key == key else { return fallback }
        let words = [n.title, n.reason].filter { !$0.isEmpty }.joined(separator: " ")
        guard !words.isEmpty, needs.isEmpty || words.contains(needs) else { return fallback }
        return words
    }

    /// The hub's backstop, said once over DSPi Link (`guard.hub_backstop`).
    // -- WP P1EFIX -- the row's reason alone, as the web, Qt and Android (review p1e-a F1)
    static var hubBackstop: String {
        let n = noticeForKey(key: hubBackstopKey, detail: "")
        return n.key == hubBackstopKey && !n.reason.isEmpty ? n.reason : fallbackHubBackstop
    }

    /// The countdown's line for `remainingMs`: the whole seconds left.
    static func countdownLine(_ words: String, remainingMs: UInt32) -> String {
        words.replacingOccurrences(of: "{s}", with: String((UInt64(remainingMs) + 999) / 1000))
    }

    // MARK: Derived state

    var guardWaiting: Bool { guardState.phase == .waiting }
    /// The toggle works now: a compare runs, no guard waits, nothing is under way.
    var canToggle: Bool { compare.active && !guardWaiting && toggling == 0 && !busy }

    /// The safe-apply bar shows: while a guard waits; while it is lost (Revert
    /// stays offered until the user acts, never hidden by a timer); and the
    /// end of one this console saw waiting, until the user's OK.
    var barShown: Bool {
        switch guardState.phase {
        case .waiting: return true
        case .idle: return false
        case .lost: return dismissedEnd != guardState.id
        default:
            return guardState.id == watchedGuard && dismissedEnd != guardState.id && !guardState.message.isEmpty
        }
    }

    /// Revert is offered: while waiting, and after a lost guard until the
    /// user acts, over a hub too (the core asks the hub how it ended; as Qt,
    /// and the P1-E decision; WP P1EFIX, review p1e-b F2).
    var canRevert: Bool { Self.offersRevert(guardState) }

    /// Whether the bar offers Revert for `g` (hub or not).
    static func offersRevert(_ g: GuardedApply) -> Bool {
        g.phase == .waiting || g.phase == .lost
    }

    // MARK: Connection

    private var source: CoreStateSource? { vm?.transport.coreStateSource }

    private func connectionChanged(_ connected: Bool) {
        let isAvailable = connected && source != nil
        if available != isAvailable { available = isAvailable }
        guard isAvailable else {
            if !snapshots.isEmpty { snapshots = [] }
            if compare.active { compare = Self.noCompare; abRows = [] }
            if offline != nil { offline = nil }
            return
        }
        nameFolder()
        reload()
        reloadCompare()
        run({ core -> (Bool, String, GuardedApply) in
            (core.applyGuardDefault(), core.offlineDocument(), core.applyStatus())
        }) { [weak self] result in
            guard let self = self, case .success(let (on, offlineJSON, g)) = result else { return }
            if self.guardDefault != on { self.guardDefault = on }
            self.offline = OfflineDocumentInfo.parse(offlineJSON)
            // A lost apply: ask how it ended now the device is back.
            if g.phase == .lost || self.guardState.phase == .lost { self.pollNow() }
        }
    }

    /// Name the store's folder once per core.
    private func nameFolder() {
        guard let folder = folder, let source = source else { return }
        let key = ObjectIdentifier(source)
        guard !folderSet.contains(key) else { return }
        folderSet.insert(key)
        let path = folder.path
        run({ try $0.snapshotSetDir(dir: path) }) { [weak self] result in
            if case .failure(let e) = result {
                self?.folderSet.remove(key)
                self?.post(Self.words(e), .error)
            }
        }
    }

    // MARK: Calls

    /// `body` on the core's queue, off main; `done` on main.
    /// `counts`: the buttons wait for it (`busy`); the guard's poll does not.
    private func run<T>(counts: Bool = true, _ body: @escaping (DspiCore) throws -> T,
                        done: @escaping (Result<T, DspiError>) -> Void) {
        guard let source = source else {
            done(.failure(.Disconnected))
            return
        }
        if counts { track(1) }
        CoreSnapshots.work.async {
            let result = source.call(body)
            DispatchQueue.main.async {
                if counts { self.track(-1) }
                done(result)
            }
        }
    }

    private func track(_ delta: Int) {
        running += delta
        let now = running > 0
        if busy != now { busy = now }
    }

    /// The device's state and the journal again, after a bulk image (a
    /// recall, a toggle, a guarded apply or its end): the view model shows
    /// what the device holds, and Undo is off (the history ended).
    private func refreshFromCore() {
        guard let vm = vm, let source = source else { return }
        CoreSnapshots.work.async {
            vm.applyCoreReread(source)
            let journal = source.readJournal()
            DispatchQueue.main.async { vm.undo.show(journal) }
        }
    }

    /// The user's words for a failure.
    static func words(_ error: DspiError) -> String {
        // -- WP P1SW2 -- the lock, the limits, a missing token: the core's row
        if CoreInstaller.isNamedRefusal(error) { return CoreInstaller.line(for: error) }
        let raw: String
        switch error {
        case .InvalidArgument(let msg), .Io(let msg), .Transfer(let msg): raw = msg
        case .Link(_, let msg): raw = msg
        case .Disconnected: raw = "No device connected"
        case .ReadOnly: raw = "The device is read-only"
        case .Unsupported: raw = "Not available here"
        case .Locked: raw = DeviceLock.lockedMessage
        case .Denied: raw = "This console may not change the device"
        default: raw = String(describing: error)
        }
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        while text.hasSuffix(".") { text.removeLast() }
        guard let first = text.first else { return "That did not work." }
        return first.uppercased() + String(text.dropFirst()) + "."
    }

    /// Show `text` as the notice line for a while.
    func post(_ text: String, _ kind: SnapshotsNotice.Kind = .info) {
        noticeClear?.cancel()
        notice = SnapshotsNotice(text: text, kind: kind)
        let clear = DispatchWorkItem { [weak self] in self?.notice = nil }
        noticeClear = clear
        DispatchQueue.main.asyncAfter(deadline: .now() + (kind == .info ? 6 : 10), execute: clear)
    }

    func clearNotice() {
        noticeClear?.cancel()
        notice = nil
    }

    static func changesLine(_ n: Int) -> String { n == 1 ? "1 change" : "\(n) changes" }

    static func recalledLine(_ r: SnapshotRecall) -> String {
        let base = "Recalled \u{201C}\(r.name)\u{201D}: \(changesLine(r.changes.count))."
        return r.notes.isEmpty ? base : base + " " + r.notes.joined(separator: " ")
    }

    // MARK: The store

    /// Read the open device's list again.
    func reload() {
        guard available else { return }
        run({ core in core.snapshotList(serial: "") }) { [weak self] result in
            guard let self = self, case .success(let list) = result else { return }
            self.snapshots = list.sorted { $0.savedMs > $1.savedMs }
        }
    }

    /// A default name: "Snapshot <date time>".
    static func defaultName(now: Date = Date()) -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return "Snapshot \(f.string(from: now))"
    }

    func capture(name: String, note: String = "") {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let n = trimmed.isEmpty ? Self.defaultName() : trimmed
        run({ try $0.snapshotCapture(name: n, note: note) }) { [weak self] result in
            switch result {
            case .success(let s): self?.post("Captured \u{201C}\(s.name)\u{201D}.")
            case .failure(let e): self?.post(Self.words(e), .error)
            }
            self?.reload()
        }
    }

    func rename(_ s: SnapshotInfo, to name: String) {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty, n != s.name else { return }
        run({ try $0.snapshotRename(id: s.id, name: n) }) { [weak self] result in
            if case .failure(let e) = result { self?.post(Self.words(e), .error) }
            self?.reload()
        }
    }

    func setNote(_ s: SnapshotInfo, _ note: String) {
        let n = note.trimmingCharacters(in: .whitespacesAndNewlines)
        guard n != s.note else { return }
        run({ try $0.snapshotSetNote(id: s.id, note: n) }) { [weak self] result in
            if case .failure(let e) = result { self?.post(Self.words(e), .error) }
            self?.reload()
        }
    }

    func delete(_ s: SnapshotInfo) {
        run({ try $0.snapshotDelete(id: s.id) }) { [weak self] result in
            if case .failure(let e) = result { self?.post(Self.words(e), .error) }
            self?.reload()
        }
    }

    /// Snapshot `s`'s document, for a file; nil after a notice.
    func exportText(_ s: SnapshotInfo, done: @escaping (String?) -> Void) {
        run({ try $0.snapshotExport(id: s.id) }) { [weak self] result in
            switch result {
            case .success(let text): done(text)
            case .failure(let e):
                self?.post(Self.words(e), .error)
                done(nil)
            }
        }
    }

    /// A snapshot file was picked: file it in the store.
    func importText(_ text: String) {
        run({ try $0.snapshotImport(text: text, name: "", note: "") }) { [weak self] result in
            switch result {
            case .success(let s): self?.post("Added \u{201C}\(s.name)\u{201D}.")
            case .failure(let e): self?.post(Self.words(e), .error)
            }
            self?.reload()
        }
    }

    // MARK: Recall

    /// Recall `s`: guarded (Keep or Revert) when the core guards here by
    /// default or the user asked for it; a plain recall otherwise.  Never
    /// while a guarded apply waits.
    func recall(_ s: SnapshotInfo) {
        if guardWaiting { return post(Self.finishFirst, .warning) }
        if guardDefault || rollback {
            let t = UInt32(timeoutS)
            run({ try $0.applyGuardedSnapshot(id: s.id, timeoutS: t) }) { [weak self] result in
                guard let self = self else { return }
                switch result {
                case .success(let g): self.setGuard(g, begun: true)
                case .failure(let e): self.post(Self.words(e), .error)
                }
                self.refreshFromCore()
                self.reloadCompare()
                self.reload()   // the snapshot taken before it is in the store
            }
            return
        }
        run({ try $0.snapshotRecall(id: s.id) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let r): self.post(Self.recalledLine(r), r.notes.isEmpty ? .info : .warning)
            case .failure(let e): self.post(Self.words(e), .error)
            }
            self.refreshFromCore()
            self.reloadCompare()
        }
    }

    // MARK: A/B

    private func reloadCompare() {
        run({ core in core.abState() }) { [weak self] result in
            if case .success(let c) = result { self?.setCompare(c) }
        }
    }

    private func setCompare(_ c: AbCompare) {
        let pairChanged = c.active != compare.active || c.aName != compare.aName || c.bName != compare.bName
        compare = c
        guard c.active else {
            if !abRows.isEmpty { abRows = [] }
            return
        }
        guard pairChanged || abRows.isEmpty else { return }
        run({ try $0.abDiff() }) { [weak self] result in
            guard let self = self, self.compare.active else { return }
            if case .success(let json) = result { self.abRows = SnapshotDiff.parse(json).rows }
        }
    }

    /// Start A/B: `aId` "" captures the device as it is now as A; `bId` is B.
    func startCompare(aId: String, bId: String) {
        guard !bId.isEmpty, aId != bId else { return }
        if guardWaiting { return post(Self.finishFirst, .warning) }
        run({ try $0.abStart(aId: aId, bId: bId) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let c):
                self.setCompare(c)
                self.post("Comparing: on A. Press B to switch.")
            case .failure(let e): self.post(Self.words(e), .error)
            }
            self.refreshFromCore()
        }
    }

    /// The other side: one bulk write, never guarded.
    func toggle() {
        guard compare.active else { return }
        if guardWaiting { return post(Self.finishFirst, .warning) }
        guard canToggle else { return }
        toggling += 1
        run({ try $0.abToggle() }) { [weak self] result in
            guard let self = self else { return }
            self.toggling -= 1
            switch result {
            case .success(let c):
                self.setCompare(c)
                let side = c.side == .b ? "B" : "A"
                self.post("On \(side)" + (c.recall.map { ": \(Self.changesLine($0.changes.count))." } ?? "."))
            case .failure(let e):
                self.post(Self.words(e), .error)
                self.reloadCompare()
            }
            self.refreshFromCore()
        }
    }

    /// End the compare on `side`.
    func keep(side: AbSide) {
        guard compare.active else { return }
        run({ try $0.abKeep(side: side) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let c):
                self.setCompare(c)
                self.post("Kept \(side == .b ? "B" : "A"). The compare has ended.")
            case .failure(let e): self.post(Self.words(e), .error)
            }
            self.refreshFromCore()
        }
    }

    // MARK: The safe apply

    /// Import `text` with a way back (the import preview's "Apply with
    /// rollback"): the blocks in `options`, the EQ of `eqChannels`.
    func beginGuardedImport(_ text: String, options: UInt32, eqChannels: UInt32 = allEqChannels,
                            done: @escaping (Bool) -> Void = { _ in }) {
        if guardWaiting {
            post(Self.finishFirst, .warning)
            return done(false)
        }
        let t = UInt32(timeoutS)
        run({ try $0.applyGuardedImport(text: text, options: options, eqChannels: eqChannels, timeoutS: t) }) {
            [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let g):
                self.setGuard(g, begun: true)
                done(true)
            case .failure(let e):
                self.post(Self.words(e), .error)
                done(false)
            }
            self.refreshFromCore()
            self.reloadCompare()
        }
    }

    /// The bar's Keep.
    func keepChange() {
        guard guardState.phase == .waiting else { return }
        run({ try $0.applyConfirm() }) { [weak self] result in
            switch result {
            case .success(let g): self?.setGuard(g)
            case .failure(let e): self?.post(Self.words(e), .error)
            }
        }
    }

    /// The bar's Revert: the user's word, never queued.
    func revertChange() {
        run({ try $0.applyRevert() }) { [weak self] result in
            switch result {
            case .success(let g): self?.setGuard(g)
            case .failure(let e): self?.post(Self.words(e), .error)
            }
        }
    }

    /// The bar's OK after the end: it hides.
    func dismissEnd() { dismissedEnd = guardState.id }

    /// Drive the guard once now (a lost one: the hub's word after a reconnect).
    func pollNow() {
        guard !polling, source != nil else { return }
        polling = true
        run(counts: false, { core in core.applyPoll() }) { [weak self] result in
            self?.polling = false
            if case .success(let g) = result { self?.setGuard(g) }
        }
    }

    private func setGuard(_ g: GuardedApply, begun: Bool = false) {
        let before = guardState
        guardState = g
        if g.phase == .waiting { watchedGuard = g.id }
        if begun, g.phase != .waiting { watchedGuard = g.id }   // an apply that failed and was put back
        schedulePoll()
        let open: Set<GuardPhase> = [.waiting, .lost, .idle]
        let wasOpen = before.phase == .waiting || before.phase == .lost
        let ended = !open.contains(g.phase) && (g.id != before.id || (wasOpen && g.phase != before.phase))
        guard ended else { return }
        // The device may hold other settings now; a revert ended the compare.
        refreshFromCore()
        reloadCompare()
        reload()
    }

    private func schedulePoll() {
        pollTimer?.invalidate()
        pollTimer = nil
        let interval: TimeInterval
        switch guardState.phase {
        case .waiting: interval = Self.pollWaiting
        case .lost: interval = Self.pollLost
        default: return
        }
        let timer = Timer(timeInterval: interval, repeats: true) { [weak self] _ in self?.pollNow() }
        RunLoop.main.add(timer, forMode: .common)
        pollTimer = timer
    }

    // MARK: The bar's words

    /// The bar's first and second line.
    var barLines: (String, String) {
        let g = guardState
        switch g.phase {
        case .waiting:
            let label = g.label.isEmpty ? "The change" : g.label
            var second = Self.countdownLine(countdownWords, remainingMs: g.remainingMs)
            if g.hub { second += " " + Self.hubBackstop }
            return ("\u{201C}\(label)\u{201D}", second)
        case .lost:
            return (g.message.isEmpty ? "The connection was lost." : g.message, g.notes.joined(separator: " "))
        default:
            return (g.message, g.notes.joined(separator: " "))
        }
    }

    // MARK: The A/B key (macOS)

    #if os(macOS)
    /// The bare `B` key toggles A/B while a compare runs: never while a text
    /// field has the keyboard or a dialog is open (owner: plain `B`, as Qt).
    func installKeyMonitor() {
        guard keyMonitor == nil else { return }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self = self, self.compare.active,
                  Self.isBareB(event), !Self.typingOrAsked() else { return event }
            self.toggle()
            return nil
        }
    }

    static func isBareB(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection([.command, .option, .control, .shift, .function])
        return mods.isEmpty && event.charactersIgnoringModifiers?.lowercased() == "b" && !event.isARepeat
    }

    private static func typingOrAsked() -> Bool {
        if NSApp.modalWindow != nil { return true }
        guard let window = NSApp.keyWindow else { return true }
        if window.attachedSheet != nil { return true }
        return window.firstResponder is NSText
    }
    #endif
}

// MARK: - Offline documents

/// The open document (`offlineDocument`): `{offline, fileName, name, kind, platform}`.
struct OfflineDocumentInfo: Equatable {
    let fileName: String
    let name: String
    let kind: String
    let platform: String

    /// The badge's words.
    var badge: String { "File: \(fileName.isEmpty ? name : fileName)" }

    /// nil for a device (`offline: false`) or a reply that cannot be read.
    static func parse(_ json: String) -> OfflineDocumentInfo? {
        guard let o = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
              o["offline"] as? Bool == true else { return nil }
        return OfflineDocumentInfo(fileName: o["fileName"] as? String ?? "", name: o["name"] as? String ?? "",
                                   kind: o["kind"] as? String ?? "", platform: o["platform"] as? String ?? "")
    }
}

/// One template of `templateList`.
struct TemplateEntry: Identifiable, Equatable {
    let id: String
    let name: String
    let builtin: Bool
    let platform: String
    let status: String

    static func parse(_ json: String) -> [TemplateEntry] {
        let list = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [[String: Any]] ?? []
        return list.compactMap { m in
            guard let id = m["id"] as? String, !id.isEmpty else { return nil }
            return TemplateEntry(id: id, name: m["name"] as? String ?? id, builtin: m["builtin"] as? Bool ?? false,
                                 platform: m["platform"] as? String ?? "", status: m["status"] as? String ?? "")
        }
    }

    /// A file name for template `name`: what a file system takes, the
    /// document's extension added.
    static func fileName(_ name: String) -> String {
        let bad = CharacterSet(charactersIn: "\\/:*?\"<>|").union(.controlCharacters)
        var base = String(name.unicodeScalars.map { bad.contains($0) ? "_" : Character($0) })
        base = base.trimmingCharacters(in: CharacterSet(charactersIn: ". ").union(.whitespaces))
        if base.count > 80 { base = String(base.prefix(80)) }
        if base.isEmpty { base = "Template" }
        return "\(base).\(PresetDocument.fileExtension)"
    }
}
