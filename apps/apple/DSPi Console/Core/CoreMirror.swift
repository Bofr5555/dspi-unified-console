//
//  CoreMirror.swift
//  DSPi Console
//
//  WP SW8.  The view model's device state from the Rust core's `DspState`
//  (docs/SWIFT-SEAMS.md 5.2): when the device is driven through the core
//  (`CoreBackedTransport`), the bulk read is the core's `fetchAll`, and what
//  a notification changed comes from `processNotifications` and `getState`,
//  never from decoding the image or a PARAM_CHANGED offset in Swift.
//
//  Two copies, one owner each.  The core's `DspState` is the device as last
//  confirmed; the `@Published` values are what the UI shows, and may run
//  ahead of the device while an edit is in flight.  `CoreMirror.apply`
//  writes only the fields whose core value changed since it last applied
//  them, and holds back a field the user is editing (the pending-key rule):
//  a field edited locally is not overwritten until the core reports the same
//  value, 500 ms pass after the last local edit, or the connection changes.
//  So a late echo of an earlier drag position never pulls a slider back.
//
//  Older (or newer) firmware: the core maps any image it knows onto the
//  current layout and makes the connection read-only (`compat`, setters
//  return `ReadOnly`).  The view model shows the values and
//  `deviceReadOnly`; the detail controls are disabled and `ReadOnlyBanner`
//  says why.
//
//  Unsaved changes (-- WP SWF6 --).  Through the core, the preset baseline
//  and the change list are the core's (`presetChanges`, CG7's sentences in
//  the Mac's words; CG33's rule keeps an edit made right after a preset load
//  unsaved).  The view model holds the last list in `corePresetChanges`,
//  delivered on main with each state change, after a fetch, a load and a
//  save; `hasUnsavedChanges` and `computeDiff` read it.  The transports not
//  yet on the core keep the Swift snapshot (PresetSnapshot.swift).
//
//  Threads.  Core calls go through the source, on its `coreQueue`; `apply`
//  runs on main.
//

import Foundation
import Combine
import SwiftUI
import os
import dspi_core
import dspi_constants

// MARK: - The core's state, from a transport

/// A transport whose device state the Rust core keeps.
protocol CoreStateSource: AnyObject {
    /// `fetchAll()`, then `getState()`, on the core's queue.  Blocking: call
    /// off the main thread.
    func readAll() -> Result<DspState, DspiError>
    /// `getState()` on the core's queue (no device I/O).
    func readState() -> DspState
    /// Each `processNotifications` result that changed the mirror, with the
    /// state after it.  Delivered on the main thread.
    func addStateObserver(_ handler: @escaping (CoreStateChange) -> Void) -> AnyCancellable
    // -- WP SWF6 --
    /// `presetChanges()` on the core's queue (no device I/O).  Blocking:
    /// call off the main thread.
    func readPresetChanges() -> [String]
    /// The live state is now the active preset (a save the app made): apply
    /// the queued notifications, so the last edits' echoes are in it, then
    /// `captureBaseline()`.  Returns the change list after.  Blocking: call
    /// off the main thread.
    func capturePresetBaseline() -> [String]
    /// The core's `loadPreset`: load, confirm, re-read, and make the preset
    /// the baseline.  The state and the change list after it.  Blocking:
    /// call off the main thread.
    func loadPreset(slot: UInt8) -> Result<(DspState, [String]), DspiError>
    // -- WP SWF7 --
    /// Before an unsaved-changes prompt (c70-N1): when a raw SET went out
    /// less than `CoreBackedTransport.settleWindow` ago, its echo may not be
    /// in the core's list yet, so apply the queued notifications and return
    /// the list after; nil when no SET is that recent (the list is current).
    /// Blocks on the core's queue only in that case; main thread.
    /// -- WP SWF8 -- At most `settleWindow` in all (nil past it), and nil at
    /// once over a hub, whose echoes reach the core through main.
    func settlePresetChanges() -> [String]?
    // -- WP SWF8 -- c75-N3
    /// The core's `maxDelayMs()` (no device I/O).  Blocking: call off the
    /// main thread.
    func readMaxDelayMs() -> Float?
    /// For a delay past the line (or NaN): the core's refusal of it, from
    /// its typed setter, which sends nothing then; nil when the core would
    /// take it or refused for another reason.  `reply` runs on main.
    func outputDelayRefusal(output: UInt8, ms: Float, reply: @escaping (String?) -> Void)
    // -- WP SWMAC1 -- The core's change journal (CoreUndo.swift).
    /// `canUndo()` and `canRedo()` on the core's queue (no device I/O).
    /// Blocking: call off the main thread.
    func readJournal() -> CoreJournalState
    /// The core's `undo()` or `redo()`, with the state, the change list and
    /// the journal after it.  Blocking: call off the main thread.
    func stepJournal(redo: Bool) -> CoreJournalStep
    /// The core's `setDragging`, queued behind the writes sent before it.
    func setDragging(_ dragging: Bool)
    // -- WP SW9 -- The domains' typed calls (SWIFT-SEAMS 5.2).
    /// A typed setter on the core's queue, behind every write sent before
    /// it; never replayed.  `lane`: the slider fast path, where a newer
    /// value for the same key replaces one still queued.
    func write(lane: String?, _ body: @escaping (DspiCore) throws -> Void)
    /// -- WP SW9 -- fixup sw9 F2: the committed value of a lane's setting
    /// (a release, a typed value): it supersedes the value still queued for
    /// `lane`, and a value issued after it is sent after it.
    func commit(lane: String, _ body: @escaping (DspiCore) throws -> Void)
    /// A typed read (or a write with a reply) on the core's queue.  Blocking:
    /// call off the main thread.
    func call<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError>
    // -- WP SW11 --
    /// A typed write with a reply (a pin setter's status) on the core's
    /// queue, behind every write sent before it, then the state published as
    /// after a queued write.  Never replayed.  Blocking: call off the main
    /// thread.
    func writeAndWait<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError>
    // -- WP SW12 --
    /// A typed re-read (no write) on the core's queue, behind every write
    /// sent before it, then the state published as after a queued write.
    /// No write time is stamped and no refusal posted (review sw11 N2).
    /// Blocking: call off the main thread.
    func readAndPublish<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError>
    /// -- WP SW12 -- fixup sw12 F2: a write the core refused in a JSON reply
    /// (`csApply`): show `message`; `resync`: re-read the device too, as for
    /// a refused typed write.  Any thread.
    func postRefusal(_ message: String, resync: Bool)
}

extension CoreStateSource {
    func settlePresetChanges() -> [String]? { nil }
    func readMaxDelayMs() -> Float? { nil }
    func outputDelayRefusal(output: UInt8, ms: Float, reply: @escaping (String?) -> Void) {
        DispatchQueue.main.async { reply(nil) }
    }
    // -- WP SWMAC1 -- A source without a journal: nothing to undo.
    func readJournal() -> CoreJournalState { CoreJournalState() }
    func stepJournal(redo: Bool) -> CoreJournalStep {
        CoreJournalStep(result: .failure(.Unsupported), state: readState(), presetChanges: readPresetChanges(),
                        journal: CoreJournalState())
    }
    func setDragging(_ dragging: Bool) {}
    // -- WP SW9 -- A source without a core takes no typed call.
    func write(lane: String?, _ body: @escaping (DspiCore) throws -> Void) {}
    func commit(lane: String, _ body: @escaping (DspiCore) throws -> Void) {}
    func call<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> { .failure(.Unsupported) }
    func writeAndWait<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> { .failure(.Unsupported) }   // -- WP SW11 --
    func readAndPublish<T>(_ body: (DspiCore) throws -> T) -> Result<T, DspiError> { .failure(.Unsupported) }   // -- WP SW12 --
    func postRefusal(_ message: String, resync: Bool) { DeviceLock.notice.post(message) }   // -- WP SW12 --
}

/// What one `processNotifications` changed, and the core's state after it.
struct CoreStateChange {
    let flags: UInt32
    let state: DspState
    /// The core's unsaved-changes list after the change (-- WP SWF6 --);
    /// nil when it was not read.
    var presetChanges: [String]? = nil
    /// Whether the core can undo and redo after the change (-- WP SWMAC1 --);
    /// nil when it was not read.
    var journal: CoreJournalState? = nil
    /// -- WP SW9 -- False for the state published after this app's own typed
    /// writes: the core patched its mirror with the value sent, which the
    /// device has not confirmed, so it never ends a pending edit (a late
    /// echo of an earlier value may still follow).
    var confirms = true

    /// The `NotifyResult` flags after which the mirror is read again.
    static let stateFlags = DspiConstants.NOTIFY_STATE | DspiConstants.NOTIFY_REFRESHED
        | DspiConstants.NOTIFY_PRESET | DspiConstants.NOTIFY_NAMES
        | DspiConstants.NOTIFY_STREAMS   // -- WP SW11 -- (CG25)

    var refreshed: Bool { flags & DspiConstants.NOTIFY_REFRESHED != 0 }
    var presetLoaded: Bool { flags & DspiConstants.NOTIFY_PRESET != 0 }
}

/// Fans state changes out to observers on the main thread.
final class CoreStateFanout {
    private var handlers: [UUID: (CoreStateChange) -> Void] = [:]
    private let lock = NSLock()

    func add(_ handler: @escaping (CoreStateChange) -> Void) -> AnyCancellable {
        let id = UUID()
        lock.lock(); handlers[id] = handler; lock.unlock()
        return AnyCancellable { [weak self] in
            guard let self = self else { return }
            self.lock.lock(); self.handlers.removeValue(forKey: id); self.lock.unlock()
        }
    }

    var isEmpty: Bool { lock.lock(); defer { lock.unlock() }; return handlers.isEmpty }

    func publish(_ change: CoreStateChange) {
        lock.lock(); let snapshot = Array(handlers.values); lock.unlock()
        guard !snapshot.isEmpty else { return }
        if Thread.isMainThread {
            snapshot.forEach { $0(change) }
        } else {
            DispatchQueue.main.async { snapshot.forEach { $0(change) } }
        }
    }
}

extension DeviceTransport {
    /// The core behind this transport, if the device is driven through one.
    var coreStateSource: CoreStateSource? { (self as? CoreStateSourceProviding)?.providedCoreStateSource }
}

/// A transport that can name the core of the device it drives now: the core
/// transport itself, or a set or composite forwarding to its active member.
protocol CoreStateSourceProviding: AnyObject {
    var providedCoreStateSource: CoreStateSource? { get }
}

// MARK: - The mirror

/// Applies `DspState` to a `DSPViewModel`, field by field, with the
/// pending-key rule.  Main thread only.
final class CoreMirror {
    /// How long a locally edited field ignores core values that differ from
    /// it (SWIFT-SEAMS 5.2).
    static let defaultPendingWindow: TimeInterval = 0.5
    /// The window in use (tests set 0 to show what the rule prevents).
    var pendingWindow: TimeInterval = CoreMirror.defaultPendingWindow

    private static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "CoreMirror")

    /// The clock of the pending window (tests move it).
    var now: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }

    private var fields: [MirrorField] = []
    private var watches: Set<AnyCancellable> = []
    private var stateObserver: AnyCancellable?
    private weak var source: CoreStateSource?
    private weak var vm: DSPViewModel?
    /// True while `apply` writes, so its own writes are not taken for edits.
    private var applying = false
    private var sweep: DispatchWorkItem?
    /// The last state applied, for the sweep after a pending window.
    private(set) var latest: DspState?

    /// Counts the sources followed (-- WP SWF4 --): a state change or a
    /// revert read under an earlier one is dropped.
    private(set) var attachment: UInt64 = 0

    /// The source the view model follows now, if any.
    var attachedSource: CoreStateSource? { source }

    init(vm: DSPViewModel) {
        self.vm = vm
        fields = CoreMirror.fields()
        for field in fields {
            field.watch(vm, isEdit: { [weak self] in self?.applying == false },
                        clock: { [weak self] in self?.now() ?? ProcessInfo.processInfo.systemUptime })
                .forEach { watches.insert($0) }
        }
    }

    /// Follow `source` (nil: none).  Clears every pending key and what was
    /// applied, so the next state is applied whole: a new connection.
    func attach(_ source: CoreStateSource?) {
        if source !== self.source || source == nil { reset() }
        guard source !== self.source else { return }
        self.source = source
        // -- WP SWF4 -- A change carries the attachment it came from: one the
        // core published a main-queue hop before a member switch (or a
        // disconnect) is dropped, not applied to the next device's view.
        attachment &+= 1
        let attached = attachment
        stateObserver = source?.addStateObserver { [weak self] change in
            guard let self = self, self.attachment == attached else { return }
            self.changed(change)
        }
    }

    /// Forget pending keys and applied values (a new connection).
    func reset() {
        fields.forEach { $0.reset() }
        latest = nil
        sweep?.cancel(); sweep = nil
        if vm?.corePresetChanges.isEmpty == false { vm?.corePresetChanges = [] }   // -- WP SWF6 --
        vm?.undo.show(CoreJournalState())   // -- WP SWMAC1 --
    }

    /// The core reported a change.
    private func changed(_ change: CoreStateChange) {
        guard let vm = vm else { return }
        vm.applyCoreState(change.state, force: change.refreshed, confirms: change.confirms)   // -- WP SW9 --
        // -- WP SWF6 -- No Swift snapshot at a preset's refresh: the core
        // rebased its baseline when it re-read, keeping what changed after the
        // load unsaved (CG33), and its list comes with the change.
        if let changes = change.presetChanges { showPresetChanges(changes, from: source) }
        if let journal = change.journal { vm.undo.show(journal) }   // -- WP SWMAC1 --
        vm.followCoreEvents(change.flags)   // -- WP SW11 --
    }

    // -- WP SWMAC1 --
    /// Undo (or redo) through the core's journal of the source followed now,
    /// then put the state after it on screen.  `done` runs on main with the
    /// step's result, nil when the view follows another source by then.
    /// False, and `done` never runs, without a source.
    @discardableResult
    func stepJournal(redo: Bool, done: @escaping (Result<HistoryMove, DspiError>?) -> Void) -> Bool {
        guard let source = source else { return false }
        let attached = attachment
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let step = source.stepJournal(redo: redo)
            DispatchQueue.main.async {
                guard let self = self, self.attachment == attached, let vm = self.vm else { done(nil); return }
                // The step wrote the value back: it wins over a pending edit.
                vm.applyCoreState(step.state, force: true)
                self.showPresetChanges(step.presetChanges, from: source)
                vm.undo.show(step.journal)
                done(step.result)
            }
        }
        return true
    }

    // -- WP SWF6 --
    /// Show the core's unsaved-changes list, read from `source`: dropped when
    /// the view model follows another source by now.  Main thread.
    func showPresetChanges(_ changes: [String], from source: CoreStateSource?) {
        guard let vm = vm, let source = source, source === self.source else { return }
        if vm.corePresetChanges != changes { vm.corePresetChanges = changes }
    }

    /// Put the core's current state back on screen, pending edits included:
    /// a refused write left the device as it was (RefusedWrite's resync).
    func revertToCore() {
        guard let source = source else { return }
        let attached = attachment
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let state = source.readState()
            DispatchQueue.main.async {
                // -- WP SWF4 -- Only onto the view of the device it was read from.
                guard let self = self, self.attachment == attached else { return }
                self.vm?.applyCoreState(state, force: true)
            }
        }
    }

    /// Write `state` into the view model.  `force` writes every field,
    /// pending or not (a whole re-read: connect, preset load, refusal).
    /// Returns the keys written.
    @discardableResult
    func apply(_ state: DspState, force: Bool, confirms: Bool = true) -> [String] {
        guard let vm = vm else { return [] }
        latest = state
        applying = true
        defer { applying = false }
        let t = now()
        var written: [String] = []
        for field in fields where field.apply(state, vm: vm, now: t, window: pendingWindow, force: force,
                                              confirms: confirms) {
            written.append(field.key)
        }
        scheduleSweep(t)
        return written
    }

    /// Keys held back by a pending edit, for tests.
    var deferredKeys: [String] { fields.filter(\.isDeferred).map(\.key) }

    /// After the earliest pending window ends, apply what the core said
    /// meanwhile to the fields it held back.
    private func scheduleSweep(_ t: TimeInterval) {
        sweep?.cancel(); sweep = nil
        let ends = fields.compactMap { $0.deferredUntil(window: pendingWindow) }
        guard let first = ends.min(), let state = latest else { return }
        let work = DispatchWorkItem { [weak self] in
            guard let self = self else { return }
            self.sweep = nil
            self.vm?.applyCoreState(state, force: false)
        }
        sweep = work
        DispatchQueue.main.asyncAfter(deadline: .now() + max(0.01, first - t + 0.01), execute: work)
    }
}

// MARK: - Fields

private protocol MirrorField: AnyObject {
    var key: String { get }
    var isDeferred: Bool { get }
    func apply(_ state: DspState, vm: DSPViewModel, now: TimeInterval, window: TimeInterval, force: Bool,
               confirms: Bool) -> Bool
    func watch(_ vm: DSPViewModel, isEdit: @escaping () -> Bool,
               clock: @escaping () -> TimeInterval) -> [AnyCancellable]
    func deferredUntil(window: TimeInterval) -> TimeInterval?
    func reset()
}

/// One mirrored value: how the core state gives it, how the view model
/// holds it, and its pending edit.
private final class Field<V: Equatable>: MirrorField {
    let key: String
    private let value: (DspState, DSPViewModel) -> V
    private let get: (DSPViewModel) -> V
    private let set: (DSPViewModel, V) -> Void
    private let edits: ((DSPViewModel) -> AnyPublisher<V, Never>)?

    /// The core value last applied (or held back).
    private var last: V?
    /// The value of the last local edit, and when it was made.
    private var pending: (value: V, at: TimeInterval)?
    private(set) var isDeferred = false

    init(_ key: String,
         value: @escaping (DspState, DSPViewModel) -> V,
         get: @escaping (DSPViewModel) -> V,
         set: @escaping (DSPViewModel, V) -> Void,
         edits: ((DSPViewModel) -> AnyPublisher<V, Never>)?) {
        self.key = key
        self.value = value
        self.get = get
        self.set = set
        self.edits = edits
    }

    func apply(_ state: DspState, vm: DSPViewModel, now: TimeInterval, window: TimeInterval, force: Bool,
               confirms: Bool) -> Bool {
        let core = value(state, vm)
        if !force, !isDeferred, let last = last, last == core { return false }
        last = core
        if !force, let p = pending {
            // -- WP SW9 -- Our own write's value, unconfirmed: still pending.
            if !confirms, p.value == core { isDeferred = false; return false }
            if p.value == core {
                pending = nil
            } else if now - p.at < window {
                isDeferred = true
                return false
            } else {
                pending = nil
            }
        }
        pending = nil
        isDeferred = false
        guard get(vm) != core else { return false }
        set(vm, core)
        return true
    }

    func watch(_ vm: DSPViewModel, isEdit: @escaping () -> Bool,
               clock: @escaping () -> TimeInterval) -> [AnyCancellable] {
        guard let edits = edits else { return [] }
        // A @Published publisher sends the current value first: not an edit.
        return [edits(vm).dropFirst().sink { [weak self] v in
            guard isEdit() else { return }
            self?.pending = (v, clock())
        }]
    }

    func deferredUntil(window: TimeInterval) -> TimeInterval? {
        guard isDeferred, let p = pending else { return nil }
        return p.at + window
    }

    func reset() {
        last = nil
        pending = nil
        isDeferred = false
    }
}

extension CoreMirror {
    /// Every mirrored field (SWIFT-SEAMS 5.2: the property paths the old bulk
    /// decoder wrote).
    fileprivate static func fields() -> [MirrorField] {
        var f: [MirrorField] = []

        /// A whole `@Published` property of the view model or of one of its
        /// parameter objects.
        func whole<V: Equatable>(_ key: String,
                                 _ path: ReferenceWritableKeyPath<DSPViewModel, V>,
                                 _ publisher: KeyPath<DSPViewModel, Published<V>.Publisher>,
                                 _ value: @escaping (DspState, DSPViewModel) -> V) {
            f.append(Field(key, value: value,
                           get: { $0[keyPath: path] },
                           set: { $0[keyPath: path] = $1 },
                           edits: { $0[keyPath: publisher].eraseToAnyPublisher() }))
        }

        // Volumes and global
        whole("master_volume_db", \.masterVolumeDB, \.$masterVolumeDB) { s, _ in s.masterVolumeDb }
        whole("user_volume_db", \.userVolumeDB, \.$userVolumeDB) { s, _ in s.userVolumeDb }
        whole("bypass", \.bypass, \.$bypass) { s, _ in s.bypass }
        whole("input_preamp_db", \.preampDB, \.$preampDB) { s, _ in
            let n = CoreMirror.inputs(s)
            return (0..<MAX_MATRIX_INPUTS).map { $0 < n ? (s.inputPreampDb[safe: $0] ?? 0) : 0 }
        }
        whole("channel_delays", \.channelDelays, \.$channelDelays) { s, _ in
            var d: [Int: Float] = [:]
            for i in 0..<CoreMirror.channels(s) { d[i] = s.channelDelays[safe: i] ?? 0 }
            return d
        }

        // Loudness, crossfeed
        whole("loudness_enabled", \.loudnessEnabled, \.$loudnessEnabled) { s, _ in s.loudnessEnabled }
        whole("loudness_output_mask", \.loudness.outputMask, \.loudness.$outputMask) { s, _ in s.loudnessOutputMask }
        whole("loudness_ref_spl", \.loudness.refSPL, \.loudness.$refSPL) { s, _ in s.loudnessRefSpl }
        whole("loudness_intensity", \.loudness.intensity, \.loudness.$intensity) { s, _ in s.loudnessIntensity }
        whole("crossfeed_enabled", \.crossfeedEnabled, \.$crossfeedEnabled) { s, _ in s.crossfeedEnabled }
        whole("crossfeed_preset", \.crossfeed.preset, \.crossfeed.$preset) { s, _ in Int(s.crossfeedPreset) }
        whole("crossfeed_itd", \.crossfeed.itd, \.crossfeed.$itd) { s, _ in s.crossfeedItd }
        whole("crossfeed_freq", \.crossfeed.freq, \.crossfeed.$freq) { s, _ in s.crossfeedFreq }
        whole("crossfeed_feed", \.crossfeed.feed, \.crossfeed.$feed) { s, _ in s.crossfeedFeed }
        whole("crossfeed_output_pair_mask", \.crossfeed.outputMask, \.crossfeed.$outputMask) { s, _ in
            s.crossfeedOutputPairMask
        }
        whole("crossfeed_pair_map", \.crossfeed.pairMap, \.crossfeed.$pairMap) { s, _ in
            CrossfeedPairMap(s.crossfeedPairMap)
        }

        // Psychoacoustic bass
        whole("psybass_enabled", \.psybassEnabled, \.$psybassEnabled) { s, _ in s.psybassEnabled }
        whole("psybass_output_mask", \.psybass.outputMask, \.psybass.$outputMask) { s, _ in s.psybassOutputMask }
        whole("psybass_cutoff_hz", \.psybass.cutoffHz, \.psybass.$cutoffHz) { s, _ in s.psybassCutoffHz }
        whole("psybass_harmonics_db", \.psybass.harmonicsDB, \.psybass.$harmonicsDB) { s, _ in s.psybassHarmonicsDb }
        whole("psybass_drive_db", \.psybass.driveDB, \.psybass.$driveDB) { s, _ in s.psybassDriveDb }
        whole("psybass_character_pct", \.psybass.characterPct, \.psybass.$characterPct) { s, _ in s.psybassCharacterPct }
        whole("psybass_original_db", \.psybass.originalDB, \.psybass.$originalDB) { s, _ in s.psybassOriginalDb }

        // Subharmonic synthesizer (`solo` is runtime only and not mirrored)
        func sh(_ id: UInt8, _ s: DspState) -> Float { s.subharm[safe: Int(id)] ?? 0 }
        whole("subharm.enabled", \.subharmEnabled, \.$subharmEnabled) { s, _ in sh(DspiConstants.SUBHARM_PARAM_ENABLED, s) != 0 }
        whole("subharm.mask", \.subharm.outputMask, \.subharm.$outputMask) { s, _ in
            UInt16(clamping: Int(sh(DspiConstants.SUBHARM_PARAM_MASK, s)))
        }
        whole("subharm.low", \.subharm.lowDB, \.subharm.$lowDB) { s, _ in sh(DspiConstants.SUBHARM_PARAM_LOW, s) }
        whole("subharm.high", \.subharm.highDB, \.subharm.$highDB) { s, _ in sh(DspiConstants.SUBHARM_PARAM_HIGH, s) }
        whole("subharm.top", \.subharm.topDB, \.subharm.$topDB) { s, _ in sh(DspiConstants.SUBHARM_PARAM_TOP, s) }
        whole("subharm.boost", \.subharm.boostDB, \.subharm.$boostDB) { s, _ in sh(DspiConstants.SUBHARM_PARAM_BOOST, s) }
        whole("subharm.select", \.subharm.selectMode, \.subharm.$selectMode) { s, _ in
            min(Int(sh(DspiConstants.SUBHARM_PARAM_SELECT, s)), SUBHARM_SELECT_SUSTAINED)
        }
        whole("subharm.depth", \.subharm.selectDepthPct, \.subharm.$selectDepthPct) { s, _ in sh(DspiConstants.SUBHARM_PARAM_DEPTH, s) }
        whole("subharm.hold", \.subharm.selectHoldMs, \.subharm.$selectHoldMs) { s, _ in sh(DspiConstants.SUBHARM_PARAM_HOLD, s) }
        whole("subharm.ceiling", \.subharm.ceilingDB, \.subharm.$ceilingDB) { s, _ in sh(DspiConstants.SUBHARM_PARAM_CEILING, s) }
        whole("subharm.link", \.subharm.linkPairs, \.subharm.$linkPairs) { s, _ in sh(DspiConstants.SUBHARM_PARAM_LINK, s) != 0 }

        // Tube modeller
        func tb(_ id: UInt8, _ s: DspState) -> Float { s.tube[safe: Int(id)] ?? 0 }
        whole("tube.enabled", \.tubeEnabled, \.$tubeEnabled) { s, _ in tb(DspiConstants.TUBE_PARAM_ENABLED, s) != 0 }
        whole("tube.mask", \.tube.outputMask, \.tube.$outputMask) { s, _ in
            UInt16(clamping: Int(tb(DspiConstants.TUBE_PARAM_MASK, s)))
        }
        whole("tube.type", \.tube.type, \.tube.$type) { s, _ in min(Int(tb(DspiConstants.TUBE_PARAM_TYPE, s)), TUBE_TYPE_MAX) }
        whole("tube.drive", \.tube.driveDB, \.tube.$driveDB) { s, _ in tb(DspiConstants.TUBE_PARAM_DRIVE, s) }
        whole("tube.bias", \.tube.biasPct, \.tube.$biasPct) { s, _ in tb(DspiConstants.TUBE_PARAM_BIAS, s) }
        whole("tube.asym", \.tube.asymDB, \.tube.$asymDB) { s, _ in tb(DspiConstants.TUBE_PARAM_ASYM, s) }
        whole("tube.hardness", \.tube.hardnessPct, \.tube.$hardnessPct) { s, _ in tb(DspiConstants.TUBE_PARAM_HARDNESS, s) }
        whole("tube.sag", \.tube.sagPct, \.tube.$sagPct) { s, _ in tb(DspiConstants.TUBE_PARAM_SAG, s) }
        whole("tube.rectifier", \.tube.rectifier, \.tube.$rectifier) { s, _ in
            min(Int(tb(DspiConstants.TUBE_PARAM_RECTIFIER, s)), TUBE_RECT_MAX)
        }
        whole("tube.xfmr", \.tube.xfmrEnabled, \.tube.$xfmrEnabled) { s, _ in tb(DspiConstants.TUBE_PARAM_XFMR, s) != 0 }
        whole("tube.damping", \.tube.xfmrDamping, \.tube.$xfmrDamping) { s, _ in tb(DspiConstants.TUBE_PARAM_DAMPING, s) }
        whole("tube.resonance", \.tube.xfmrResHz, \.tube.$xfmrResHz) { s, _ in tb(DspiConstants.TUBE_PARAM_RESONANCE, s) }
        whole("tube.mix", \.tube.mixPct, \.tube.$mixPct) { s, _ in tb(DspiConstants.TUBE_PARAM_MIX, s) }
        whole("tube.trim", \.tube.trimDB, \.tube.$trimDB) { s, _ in tb(DspiConstants.TUBE_PARAM_TRIM, s) }

        // Output limiter: one record per wire slot, slots past this device's
        // outputs read as defaults; a group that disagrees is ganged as the
        // firmware does at its next recompute.
        whole("limiter", \.limiter.outputs, \.limiter.$outputs) { s, _ in
            var outs = [LimiterOutputSettings](repeating: LimiterOutputSettings(), count: WIRE_MAX_OUTPUT_CHANNELS)
            let n = min(CoreMirror.outputs(s), WIRE_MAX_OUTPUT_CHANNELS)
            let groups = [UInt8](s.limiterLinkGroup)
            for k in 0..<n {
                outs[k] = LimiterOutputSettings(
                    enabled: s.limiterEnabled[safe: k] ?? false,
                    thresholdDB: DSPViewModel.clampLimiterThreshold(s.limiterThresholdDb[safe: k] ?? 0),
                    releaseMs: DSPViewModel.clampLimiterRelease(s.limiterReleaseMs[safe: k] ?? 0),
                    linkGroup: LimiterGang.storedGroup(groups[safe: k] ?? 0))
            }
            LimiterGang.gangAll(&outs, count: n)
            return outs
        }

        // Stereo upmixer
        func um(_ id: UInt8, _ s: DspState) -> Float { s.upmix[safe: Int(id)] ?? 0 }
        whole("upmix.enabled", \.upmixEnabled, \.$upmixEnabled) { s, _ in um(DspiConstants.UPMIX_PARAM_ENABLED, s) != 0 }
        whole("upmix.center_mode", \.upmix.centerMode, \.upmix.$centerMode) { s, _ in Int(um(DspiConstants.UPMIX_PARAM_CENTER_MODE, s)) }
        whole("upmix.surround_mode", \.upmix.surroundMode, \.upmix.$surroundMode) { s, _ in
            Int(um(DspiConstants.UPMIX_PARAM_SURROUND_MODE, s))
        }
        whole("upmix.presence", \.upmix.presenceDB, \.upmix.$presenceDB) { s, _ in um(DspiConstants.UPMIX_PARAM_PRESENCE, s) }
        whole("upmix.strength", \.upmix.strengthPct, \.upmix.$strengthPct) { s, _ in um(DspiConstants.UPMIX_PARAM_STRENGTH, s) }
        whole("upmix.width", \.upmix.centerWidthPct, \.upmix.$centerWidthPct) { s, _ in um(DspiConstants.UPMIX_PARAM_WIDTH, s) }
        whole("upmix.threshold", \.upmix.thresholdPct, \.upmix.$thresholdPct) { s, _ in um(DspiConstants.UPMIX_PARAM_THRESHOLD, s) }
        whole("upmix.attack", \.upmix.attackMs, \.upmix.$attackMs) { s, _ in um(DspiConstants.UPMIX_PARAM_ATTACK, s) }
        whole("upmix.release", \.upmix.releaseMs, \.upmix.$releaseMs) { s, _ in um(DspiConstants.UPMIX_PARAM_RELEASE, s) }
        whole("upmix.detector_hpf", \.upmix.detectorHpfHz, \.upmix.$detectorHpfHz) { s, _ in
            um(DspiConstants.UPMIX_PARAM_DETECTOR_HPF, s)
        }
        whole("upmix.surround_delay", \.upmix.surroundDelayMs, \.upmix.$surroundDelayMs) { s, _ in
            um(DspiConstants.UPMIX_PARAM_SURROUND_DELAY, s)
        }
        whole("upmix.surround_hpf", \.upmix.surroundHpfHz, \.upmix.$surroundHpfHz) { s, _ in
            um(DspiConstants.UPMIX_PARAM_SURROUND_HPF, s)
        }
        whole("upmix.surround_lpf", \.upmix.surroundLpfHz, \.upmix.$surroundLpfHz) { s, _ in
            um(DspiConstants.UPMIX_PARAM_SURROUND_LPF, s)
        }
        whole("upmix.decorrelation", \.upmix.decorrPct, \.upmix.$decorrPct) { s, _ in
            um(DspiConstants.UPMIX_PARAM_DECORRELATION, s)
        }

        // Leveller
        whole("leveller_enabled", \.levellerEnabled, \.$levellerEnabled) { s, _ in s.levellerEnabled }
        whole("leveller_amount", \.leveller.amount, \.leveller.$amount) { s, _ in s.levellerAmount }
        whole("leveller_speed", \.leveller.speed, \.leveller.$speed) { s, _ in Int(s.levellerSpeed) }
        whole("leveller_max_gain_db", \.leveller.maxGainDB, \.leveller.$maxGainDB) { s, _ in s.levellerMaxGainDb }
        whole("leveller_lookahead", \.leveller.lookahead, \.leveller.$lookahead) { s, _ in s.levellerLookahead }
        whole("leveller_gate_db", \.leveller.gateDB, \.leveller.$gateDB) { s, _ in s.levellerGateDb }
        whole("leveller_detector_mask", \.leveller.detectorMask, \.leveller.$detectorMask) { s, _ in s.levellerDetectorMask }
        whole("leveller_apply_mask", \.leveller.applyMask, \.leveller.$applyMask) { s, _ in s.levellerApplyMask }

        // Matrix [input][output], outputs: backing arrays keep their full
        // size; entries past this device's channels stay off / 0 dB.
        func matrix<T>(_ rows: [[T]], _ s: DspState, _ off: T) -> [[T]] {
            var m = Array(repeating: Array(repeating: off, count: 9), count: MAX_MATRIX_INPUTS)
            for i in 0..<CoreMirror.inputs(s) { for o in 0..<CoreMirror.outputs(s) {
                if let v = rows[safe: i]?[safe: o] { m[i][o] = v }
            } }
            return m
        }
        func outputsRow<T>(_ row: [T], _ s: DspState, _ off: T) -> [T] {
            var r = Array(repeating: off, count: 9)
            for o in 0..<CoreMirror.outputs(s) { if let v = row[safe: o] { r[o] = v } }
            return r
        }
        whole("matrix_routing", \.matrixRouting, \.$matrixRouting) { s, _ in matrix(s.matrixRouting, s, false) }
        whole("matrix_gain", \.matrixGain, \.$matrixGain) { s, _ in matrix(s.matrixGain, s, Float(0)) }
        whole("matrix_invert", \.matrixInvert, \.$matrixInvert) { s, _ in matrix(s.matrixInvert, s, false) }
        whole("output_enabled", \.outputEnabled, \.$outputEnabled) { s, _ in outputsRow(s.outputEnabled, s, false) }
        whole("output_muted", \.outputMuted, \.$outputMuted) { s, _ in outputsRow(s.outputMuted, s, false) }
        whole("output_gain_db", \.outputGainDB, \.$outputGainDB) { s, _ in outputsRow(s.outputGainDb, s, Float(0)) }
        whole("output_delay_ms", \.outputDelayMS, \.$outputDelayMS) { s, _ in outputsRow(s.outputDelayMs, s, Float(0)) }

        // Pins and clocks
        whole("output_pins", \.outputPins, \.$outputPins) { s, _ in
            var pins: [UInt8] = [6, 7, 8, 9, 10]
            let raw = [UInt8](s.outputPins)
            for i in 0..<min(Int(s.numPinOutputs), 5) { if let p = raw[safe: i] { pins[i] = p } }
            return pins
        }
        whole("output_types", \.outputSlotTypes, \.$outputSlotTypes) { s, _ in
            let raw = [UInt8](s.outputTypes)
            return (0..<4).map { raw[safe: $0] ?? 0 }
        }
        whole("i2s_bck_pin", \.i2sBckPin, \.$i2sBckPin) { s, _ in s.i2sBckPin }
        whole("mck_pin", \.mckPin, \.$mckPin) { s, _ in s.mckPin }
        whole("mck_enabled", \.mckEnabled, \.$mckEnabled) { s, _ in s.mckEnabled }
        whole("mck_multiplier", \.mckMultiplier, \.$mckMultiplier) { s, _ in s.mckMultiplier == 1 ? 256 : 128 }
        // The +1-encoded clock-pin mode doubles as the feature's presence
        // flag, and an unset slave BCK keeps the live one: `DspState` has no
        // field for either, so the image byte says (a CG candidate).
        whole("i2s_clock_pin_mode", \.i2sClockPinMode, \.$i2sClockPinMode) { s, vm in
            CoreMirror.raw(s, BULK_I2S_CLOCK_PIN_MODE_OFFSET) != 0 ? s.i2sClockPinMode : vm.i2sClockPinMode
        }
        whole("i2s_bck_pin_slave", \.i2sBckPinSlave, \.$i2sBckPinSlave) { s, vm in
            CoreMirror.raw(s, BULK_I2S_BCK_PIN_SLAVE_OFFSET) != 0 ? s.i2sBckPinSlave : vm.i2sBckPinSlave
        }

        // Inputs
        whole("input_source", \.inputSource, \.$inputSource) { s, _ in Int(s.inputSource) }
        whole("spdif_rx_pin", \.spdifRxPin, \.$spdifRxPin) { s, vm in [UInt8](s.spdifRxPins).first ?? vm.spdifRxPin }
        whole("spdif_inputs_enabled", \.spdifExtEnabled, \.$spdifExtEnabled) { s, vm in
            guard CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 11) != 0 else { return vm.spdifExtEnabled }
            return (0..<(SPDIF_RX_NUM_INPUTS - 1)).map { s.spdifInputsEnabled & (1 << UInt8($0 + 1)) != 0 }
        }
        whole("spdif_rx_pins_ext", \.spdifRxPinsExt, \.$spdifRxPinsExt) { s, vm in
            var pins = vm.spdifRxPinsExt
            guard CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 11) != 0 else { return pins }
            let core = [UInt8](s.spdifRxPins)
            for i in 0..<min(pins.count, SPDIF_RX_NUM_INPUTS - 1)
            where CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 8 + i) != 0 {
                pins[i] = core[safe: i + 1] ?? pins[i]
            }
            return pins
        }
        whole("i2s_rx_pins", \.i2sRxPins, \.$i2sRxPins) { s, vm in
            var pins = vm.i2sRxPins
            let core = [UInt8](s.i2sRxPins)
            if !pins.isEmpty, let first = core.first { pins[0] = first }
            for pair in 1..<min(4, pins.count) where CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 4 + pair) != 0 {
                pins[pair] = core[safe: pair] ?? pins[pair]
            }
            return pins
        }
        whole("i2s_input_channels", \.i2sInputChannels, \.$i2sInputChannels) { s, vm in
            CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 4) != 0 ? Int(s.i2sInputChannels) : vm.i2sInputChannels
        }
        whole("i2s_input_rate", \.i2sInputRateHz, \.$i2sInputRateHz) { s, _ in i2sRateEnumToHz(s.i2sInputRate) }
        whole("i2s_clock_mode", \.i2sClockMode, \.$i2sClockMode) { s, _ in s.i2sClockMode }
        whole("lg_sound_sync_enabled", \.lgSoundSyncEnabled, \.$lgSoundSyncEnabled) { s, _ in s.lgSoundSyncEnabled }
        whole("dac_hw_mute", \.dacHwMuteConfig, \.$dacHwMuteConfig) { s, _ in
            DacHwMuteConfig(enabled: s.dacMuteEnabled, activeLow: s.dacMuteActiveLow, pin: s.dacMutePin,
                            holdMs: s.dacMuteHoldMs, releaseMs: s.dacMuteReleaseMs)
        }

        // ADAT out and in (pin 0 on the wire is "platform default" / "unset")
        whole("adat_out_enabled", \.adatEnabled, \.$adatEnabled) { s, _ in s.adatOutEnabled }
        whole("adat_out_pin", \.adatPin, \.$adatPin) { s, _ in
            CoreMirror.raw(s, BULK_ADAT_OFFSET + 1) == 0 ? ADAT_PIN_DEFAULT : s.adatOutPin
        }
        whole("adat_input_pin", \.adatInputPin, \.$adatInputPin) { s, _ in
            s.adatInputPin == 0 ? ADAT_INPUT_PIN_UNSET : s.adatInputPin
        }
        whole("adat_input_enabled", \.adatInputEnabled, \.$adatInputEnabled) { s, vm in
            CoreMirror.raw(s, BULK_INPUT_ADAT_ENABLED_P1_OFFSET) != 0 ? s.adatInputEnabled : vm.adatInputEnabled
        }
        whole("adat_input_clock_mode", \.adatInputClockMode, \.$adatInputClockMode) { s, vm in
            guard CoreMirror.raw(s, BULK_INPUT_ADAT_CLOCK_MODE_P1_OFFSET) != 0 else { return vm.adatInputClockMode }
            return s.adatInputClockMode == 1 ? ADAT_INPUT_CLOCK_MODE_SLAVE : ADAT_INPUT_CLOCK_MODE_MASTER
        }

        // EQ and crossover, one field per wire channel so an edit on one
        // channel holds back only that channel.  `active` is UI-only and kept.
        for ch in 0..<WIRE_MAX_CHANNELS {
            f.append(Field("filters[\(ch)]",
                           value: { s, vm in
                               guard ch < CoreMirror.channels(s), let row = s.filters[safe: ch] else {
                                   return vm.channelData[ch] ?? []
                               }
                               let old = vm.channelData[ch] ?? []
                               return (0..<min(10, row.count)).map { band in
                                   var p = CoreMirror.filter(row[band])
                                   if let o = old[safe: band] { p.active = o.active }
                                   return p
                               }
                           },
                           get: { $0.channelData[ch] ?? [] },
                           set: { vm, v in vm.channelData[ch] = v; vm.recomputeMagnitudes(for: ch) },
                           edits: { vm in vm.$channelData.map { $0[ch] ?? [] }.removeDuplicates().eraseToAnyPublisher() }))
            f.append(Field("xover[\(ch)]",
                           value: { s, vm in
                               guard ch < CoreMirror.channels(s), let row = s.xover[safe: ch] else {
                                   return vm.xoverData[ch] ?? []
                               }
                               let old = vm.xoverData[ch] ?? []
                               return (0..<min(WIRE_MAX_XOVER_BANDS, row.count)).map { band in
                                   let c = row[band]
                                   var p = FilterParams(type: FilterType(rawValue: Int(c.filterType)) ?? .flat,
                                                        freq: max(c.freq, 10), q: c.q > 0 ? c.q : 0.707,
                                                        gain: c.gain, bypass: c.bypass)
                                   if let o = old[safe: band] { p.active = o.active }
                                   return p
                               }
                           },
                           get: { $0.xoverData[ch] ?? [] },
                           set: { vm, v in vm.xoverData[ch] = v; vm.recomputeMagnitudes(for: ch) },
                           edits: { vm in vm.$xoverData.map { $0[ch] ?? [] }.removeDuplicates().eraseToAnyPublisher() }))
            f.append(Field("channel_names[\(ch)]",
                           value: { s, _ in ch < CoreMirror.channels(s) ? (s.channelNames[safe: ch] ?? "") : "" },
                           get: { $0.channelNames[safe: ch] ?? "" },
                           set: { vm, v in if vm.channelNames.indices.contains(ch) { vm.channelNames[ch] = v } },
                           edits: { vm in vm.$channelNames.map { $0[safe: ch] ?? "" }.removeDuplicates().eraseToAnyPublisher() }))
        }

        // Presets (the core reads the directory and names with the image)
        whole("preset_occupied", \.presetOccupied, \.$presetOccupied) { s, _ in s.presetOccupied }
        whole("preset_names", \.presetNames, \.$presetNames) { s, vm in
            (0..<vm.presetNames.count).map { s.presetOccupied & (1 << UInt16($0)) != 0 ? (s.presetNames[safe: $0] ?? "") : "" }
        }
        whole("active_preset_slot", \.activePresetSlot, \.$activePresetSlot) { s, _ in Int(s.activePresetSlot) }
        return f
    }

    // MARK: Helpers

    static func channels(_ s: DspState) -> Int { min(Int(s.numChannels), WIRE_MAX_CHANNELS) }
    static func outputs(_ s: DspState) -> Int { min(Int(s.numOutputChannels), 9) }
    static func inputs(_ s: DspState) -> Int {
        max(min(Int(s.numInputChannels), MAX_MATRIX_INPUTS), BASE_MATRIX_INPUTS)
    }

    /// A byte of the core's image (V32 layout), 0 when out of range.
    static func raw(_ s: DspState, _ offset: Int) -> UInt8 {
        guard offset >= 0, offset < s.bulkRaw.count else { return 0 }
        return s.bulkRaw[s.bulkRaw.startIndex + offset]
    }

    /// True when the core decoded an image onto this build's layout (also a
    /// read-only one); false for an unmapped wire format, whose mirror holds
    /// only defaults.
    static func decoded(_ s: DspState) -> Bool {
        s.bulkRaw.count >= Int(BULK_PARAMS_SIZE) && raw(s, 0) == UInt8(WIRE_FORMAT_VERSION)
    }

    static func platformName(_ id: UInt8) -> String {
        switch id {
        case 1: return "RP2350"
        case 2: return "STM32H723"
        default: return "RP2040"
        }
    }

    /// A PEQ band as the app holds it.  qp only for the Linkwitz Transform.
    static func filter(_ c: dspi_core.FilterParams) -> FilterParams {
        let type = FilterType(rawValue: Int(c.filterType)) ?? .flat
        var p = FilterParams(type: type, freq: c.freq, q: c.q, gain: c.gain, bypass: c.bypass)
        if type == .linkwitzTransform { p.qp = c.qp }
        return p
    }
}

private extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

// MARK: - The view model over the core

extension DSPViewModel {
    /// The bulk read through the core: `fetchAll`, then the state into the
    /// view model.  An older or newer firmware's image is shown read-only;
    /// an image the core cannot map shows nothing (false), as before.
    /// Blocking: off the main thread.
    func fetchAllParamsThroughCore(_ source: CoreStateSource) -> Bool {
        let generation = transport.generation
        switch source.readAll() {
        case .failure(let error):
            CoreMirrorLog.log.info("fetchAll failed: \(String(describing: error), privacy: .public)")
            return false
        case .success(let state):
            guard transport.generation == generation else { return false }
            let decoded = CoreMirror.decoded(state)
            // -- WP SWF6 -- `fetchAll` made this state the core's baseline.
            let changes = source.readPresetChanges()
            let maxDelay = source.readMaxDelayMs()   // -- WP SWF8 --
            DispatchQueue.main.async {
                if self.coreMaxDelayMS != maxDelay { self.coreMaxDelayMS = maxDelay }
                self.applyCoreIdentity(state)
                guard decoded else { return }
                self.applyCoreState(state, force: true)
                self.coreFetchTail(changes, from: source)
            }
            return decoded
        }
    }

    // -- WP SWF6 --
    /// `loadPreset` through the core (its `loadPreset`: the device lock, the
    /// confirmation, the re-read and the baseline), then the state into the
    /// view model.  The preset status the command layer returns: a refusal's
    /// own byte, `PRESET_STATUS_LOCKED` for the hub's lock, 0xFF for any other
    /// failure.  Blocking: off the main thread.
    func loadPresetThroughCore(_ source: CoreStateSource, slot: Int) -> UInt8 {
        guard let wire = UInt8(exactly: slot) else { return PRESET_ERR_INVALID_SLOT }
        let generation = transport.generation
        switch source.loadPreset(slot: wire) {
        case .failure(let error):
            CoreMirrorLog.log.info("loadPreset(\(slot)) failed: \(String(describing: error), privacy: .public)")
            switch error {
            case .Refused(let status): return status
            case .Locked, .Denied: return PRESET_STATUS_LOCKED
            default: return 0xFF
            }
        case .success(let (state, changes)):
            guard transport.generation == generation else { return PRESET_OK }
            DispatchQueue.main.async {
                // The new slot at once, as the raw path does (the popup's
                // check mark), then the preset's state.
                if self.activePresetSlot != slot { self.activePresetSlot = slot }
                self.applyCoreState(state, force: true)
                self.coreFetchTail(changes, from: source)
            }
            return PRESET_OK
        }
    }

    /// Firmware identity, wire format and read-only mode.  Main thread.
    func applyCoreIdentity(_ s: DspState) {
        let name = CoreMirror.platformName(s.platformId)
        if platformName != name { platformName = name }
        let inputs = CoreMirror.inputs(s)
        if numInputChannels != inputs { numInputChannels = inputs }
        if firmwareWireFormatVersion != Int(s.formatVersion) { firmwareWireFormatVersion = Int(s.formatVersion) }
        if s.fwMajor != 0 || s.fwMinor != 0 || s.fwPatch != 0 {
            let v = (major: Int(s.fwMajor), minor: Int(s.fwMinor), patch: Int(s.fwPatch), beta: Int(s.fwBeta))
            if firmwareVersion.map({ $0 != v }) ?? true { firmwareVersion = v }
        }
        let readOnly = s.compat != DspiConstants.COMPAT_OK
        if deviceReadOnly != readOnly { deviceReadOnly = readOnly }
    }

    /// Apply the core's state through the mirror, and the feature gates the
    /// image implies.  Main thread.
    func applyCoreState(_ s: DspState, force: Bool, confirms: Bool = true) {
        guard CoreMirror.decoded(s) else { return }
        let written = Set(coreMirror.apply(s, force: force, confirms: confirms))
        if force {
            // Gates: what a whole image says the firmware has (as the old
            // bulk apply did; the per-item reads after it may refine them).
            let v = Int(s.formatVersion)
            func gate(_ path: ReferenceWritableKeyPath<DSPViewModel, Bool>, _ on: Bool) {
                if self[keyPath: path] != on { self[keyPath: path] = on }
            }
            gate(\.inputSourceSupported, true)
            gate(\.i2sInputSupported, true)
            gate(\.lgSoundSyncSupported, true)
            gate(\.dacHwMuteSupported, true)
            gate(\.firmwareSupportsCrossover, true)
            gate(\.i2sClockModeSupported, v >= 21)
            gate(\.adatSupported, platformName == "RP2350")
            gate(\.adatInputSupported, platformName == "RP2350")
            if CoreMirror.raw(s, BULK_I2S_CLOCK_PIN_MODE_OFFSET) != 0 { gate(\.i2sClockPinModeSupported, true) }
            if CoreMirror.raw(s, BULK_INPUT_CONFIG_OFFSET + 11) != 0 {
                gate(\.multiSpdifSupported, true)
                // The count is the 0xEF inventory's; assume the full count until it lands.
                if spdifInputCount < 2 { spdifInputCount = SPDIF_RX_NUM_INPUTS }
            }
        }
        // The status records that carry a stored setting follow it.
        func follows(_ key: String) -> Bool { force || written.contains(key) }
        if follows("i2s_clock_mode"), i2sSlaveStatus.clockMode != i2sClockMode { i2sSlaveStatus.clockMode = i2sClockMode }
        if follows("adat_out_enabled"), adatStatus.enabled != adatEnabled { adatStatus.enabled = adatEnabled }
        if follows("adat_out_pin"), adatStatus.pin != adatPin { adatStatus.pin = adatPin }
        if follows("adat_input_pin"), adatInputStatus.pin != adatInputPin { adatInputStatus.pin = adatInputPin }
        if follows("adat_input_enabled"), adatInputStatus.enabled != adatInputEnabled {
            adatInputStatus.enabled = adatInputEnabled
        }
        if follows("adat_input_clock_mode"), adatInputStatus.clockMode != adatInputClockMode {
            adatInputStatus.clockMode = adatInputClockMode
        }
        if !written.isEmpty { CoreMirrorLog.log.debug("mirror wrote \(written.count) fields") }
    }

    /// What the old bulk apply ended with: the dashboard's visibility for
    /// this channel set, the curves, and the unsaved-changes list (the
    /// core's, after the read made its baseline).
    private func coreFetchTail(_ changes: [String], from source: CoreStateSource) {
        savedOverviewVisibility = nil
        if isOverviewMode {
            for i in 0..<chOut1 { channelVisibility[i] = (i < effectiveInputChannels) }
            for outputIdx in 0..<numOutputChannels {
                channelVisibility[eqChannel(forOutput: outputIdx)] = outputEnabled[outputIdx]
            }
        }
        recomputeAllMagnitudes()
        coreMirror.showPresetChanges(changes, from: source)   // -- WP SWF6 --
    }
}

enum CoreMirrorLog {
    static let log = Logger(subsystem: AppIdentity.logSubsystem, category: "CoreMirror")
}

// MARK: - Read-only banner

/// Says why the controls are off: the device's firmware speaks another wire
/// format, so the core shows its settings but will not write them.
struct ReadOnlyBanner: View {
    @ObservedObject var vm: DSPViewModel

    var body: some View {
        if vm.isDeviceConnected && vm.deviceReadOnly {
            HStack(spacing: 10) {
                Image(systemName: "lock.fill")
                    .foregroundColor(.orange)
                Text(message)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color.orange.opacity(0.12))
            .overlay(Divider(), alignment: .bottom)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("readOnlyBanner")
        }
    }

    var message: String {   // -- WP SWF4 -- internal, for the tests
        if vm.firmwareWireFormatVersion > WIRE_FORMAT_VERSION {
            return "Read only: this device's firmware is newer than uConsole for DSPi. Update uConsole for DSPi to change its settings."
        }
        return "Read only: this device's firmware is older than uConsole for DSPi. Update the firmware to change its settings."
    }
}
