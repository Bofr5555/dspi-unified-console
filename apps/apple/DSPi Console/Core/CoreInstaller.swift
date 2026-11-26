//
//  CoreInstaller.swift
//  DSPi Console
//
//  WP P1SW2.  The installer's tools through the Rust core, as Qt's
//  `LimitsController`, Android's `Installer` and the web's installer panel
//  (P1QT2, P1AN2, P1WB2, P1EFIX): the limits profile (P1LM) with the core's
//  range table, the installer PIN and the lock (P1LK) with one prompt through
//  `LockState.heldBy`, the mode the app shows (the manifest and
//  `mode_resolve`), Protect all outputs and the outputs without a limiter
//  (P1PR, PRO1 S2).  The core is the authority for the rules and the words:
//  every refusal is shown as the core's notice row.
//
//  - Every call runs off the main thread, on the core's queue
//    (`CoreStateSource.call`); a PIN check takes 0.1 to 1 s (PBKDF2).
//  - With no PIN set nothing is locked and nothing asks for a PIN.
//  - The lock is on this console, not on the DSPi (OD-P1d): the unlock
//    prompt says so in the core's row (`installer_lock.scope`), or names the
//    hub when the hub holds the lock (`installer_lock.hub`).
//  - Only while the device is on the core; otherwise `available` is false and
//    the mode is the platform's default from the manifest.
//

import Foundation
import Combine
import SwiftUI
import dspi_core

final class CoreInstaller: ObservableObject {
    static let shared = CoreInstaller(vm: AppState.shared.viewModel,
                                      folder: CoreInstaller.appFolder(reset: CoreInstaller.resetAtLaunch))

    /// The device is on the core: the installer's tools work here.
    @Published private(set) var available = false
    /// The lock as the core has it now (an idle state with no device).
    @Published private(set) var lock = CoreInstaller.noLock
    /// The profile's state (`limitsStatus`).
    @Published private(set) var status: LimitsStatus?
    /// The profile being edited: the `limits` block of the core's document.
    @Published private(set) var draft = LimitsDraft()
    /// The mode the app shows, as the core resolved it (nil: no core yet).
    @Published private(set) var choice: ModeChoice?
    /// The outputs (0-based) on, above -10 dB and without a limiter (S2).
    @Published private(set) var withoutLimiter: [Int] = []
    /// A call is under way: the buttons wait.
    @Published private(set) var busy = false
    /// The last action's outcome in one line (no modal).
    @Published private(set) var notice: SnapshotsNotice?
    /// The recovery code of a PIN just set: shown this once.
    @Published var recoveryCode: String?
    /// The unlock prompt over the main window.
    @Published var unlockShown = false
    /// The mode switch over the main window.
    @Published var modeShown = false
    /// The user's choice of mode for the open device ("" none), as kept.
    @Published private(set) var chosen = ""

    // -- WP SW6 -- HUBADMIN: a device reached through a hub that keeps
    // limits has the hub's copy of the limits, the PIN and the pinned modes;
    // only an Admin of the hub edits it (`installer_target`).
    /// Which copy the page edits, as the core said (nil: not read yet, so
    /// nothing is editable).
    @Published private(set) var target: InstallerTarget?
    /// The hub's copy (`hub_limits_get`), when the target is the hub's.
    @Published private(set) var hub: HubAdminState?
    /// The recovery code shown is the hub's (`installer_lock.hub_recovery`).
    @Published private(set) var hubRecovery = false

    /// The mode manifest (the core's table; no I/O).
    let manifest = modeManifest()
    /// The limits' ranges (the core's one table; no I/O).
    let ranges = limitsFieldRanges()
    /// The limits' folder; nil names none (tests that never write).
    let folder: URL?

    private var vm: DSPViewModel?
    private var running = 0
    private var cancellables = Set<AnyCancellable>()
    private var folderSet = Set<ObjectIdentifier>()
    private var noticeClear: DispatchWorkItem?
    /// What to do once the unlock prompt succeeds (the mode switch's choice).
    private var afterUnlock: (() -> Void)?
    /// -- WP SW6 -- The core whose hub admin events reach this page.
    private weak var listenedSource: CoreStateSource?
    private lazy var hubEvents = HubEvents(self)

    static let work = DispatchQueue(label: "com.dspi.installer", qos: .userInitiated)

    static let noLock = LockState(serial: "", pinSet: false, unlocked: false, locked: false, installerOpen: true,
                                  covers: [], allowedWrites: [], wrongTries: 0, retryAfterMs: 0, recovered: false,
                                  note: "", contact: "", scopeNoticeKey: "", detail: "", heldBy: "", hubPinnedMode: "")

    /// Protect all outputs' ceiling, dBFS (P1PR; the other clients' -1).
    static let protectCeilingDb: Float = -1
    /// S2: "an output above -10 dB gain has no limiter".
    static let withoutLimiterAboveDb: Float = -10

    /// The platform class the manifest knows this app by.  The Mac, the
    /// iPhone and the iPad open in Advanced (the manifest's defaults; "phone"
    /// is Advanced since owner D9, 2026-11-25, as on Android).  -- FIX016 -- (REVG F9)
    static var platform: String {
        #if os(macOS)
        return "desktop"
        #else
        return "phone"
        #endif
    }

    /// `-ResetInstaller YES` (UI tests): start with no PIN and no profile.
    static var resetAtLaunch: Bool { UserDefaults.standard.bool(forKey: "ResetInstaller") }

    /// The app's limits folder ("limits" beside the snapshots).
    static func appFolder(reset: Bool = false) -> URL? {
        guard let url = CoreSnapshots.appFolder("limits") else { return nil }
        if reset { try? FileManager.default.removeItem(at: url) }
        return url
    }

    /// - Parameters:
    ///   - vm: the view model whose transport carries the core.
    ///   - folder: the limits folder (the app's own by default).
    init(vm: DSPViewModel, folder: URL?) {
        self.vm = vm
        self.folder = folder
        // `-ResetInstaller YES` forgets the kept choices.  `-UIMode` is
        // `launchMode` (this launch only, never kept: WP P1SW2F, F7).
        let d = UserDefaults.standard
        if Self.resetAtLaunch {
            for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        }
        vm.$isDeviceConnected
            .removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] connected in self?.connectionChanged(connected) }
            .store(in: &cancellables)
        vm.$selectedDevice
            .removeDuplicates { $0?.serial == $1?.serial }
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.connectionChanged(self?.vm?.isDeviceConnected ?? false) }
            .store(in: &cancellables)
    }

    // MARK: - Notice rows

    /// A notice row as one line: "Title: reason", or `fallback` when the
    /// table has no row for `key`.
    static func line(_ key: String, fallback: String = "") -> String {
        let n = noticeForKey(key: key, detail: "")
        guard n.key == key, !n.title.isEmpty else { return fallback }
        return n.reason.isEmpty ? n.title : "\(n.title): \(n.reason)"
    }

    /// A notice row's reason alone ("" when the table has none).
    static func reason(_ key: String) -> String {
        let n = noticeForKey(key: key, detail: "")
        return n.key == key ? n.reason : ""
    }

    /// The core's row for a refusal, "Title: reason", or the plain words of
    /// an error the table has no row for.
    static func line(for error: DspiError) -> String {
        guard isNamedRefusal(error) else { return CoreSnapshots.words(error) }
        let n = noticeFor(error: error)
        if !n.title.isEmpty { return n.reason.isEmpty ? n.title : "\(n.title): \(n.reason)" }
        switch error {
        case .LimitExceeded(_, let msg), .InstallerLock(_, let msg), .ConfirmationRequired(_, let msg): return msg
        case .ImageRefused(let msg), .LimitsUnknown(let msg): return msg
        default: return String(describing: error)
        }
    }

    /// The refusals the core names with a notice row (P1LM, P1LK, P1TK, P1HL).
    static func isNamedRefusal(_ error: DspiError) -> Bool {
        switch error {
        case .LimitExceeded, .InstallerLock, .ConfirmationRequired, .ImageRefused, .LimitsUnknown: return true
        default: return false
        }
    }

    // MARK: - Derived state

    /// The mode the app shows: `simple`, `advanced`, `installer` or
    /// `locked_user`.
    var mode: String {
        if let c = choice { return c.mode }
        // No core yet: the kept choice (never the Installer: its PIN is the
        // core's), else the platform's default.
        return chosen.isEmpty || chosen == "installer" ? Self.defaultMode(manifest) : chosen
    }
    /// The locked-user state: only volume, mute and presets.
    var lockedUser: Bool { mode == "locked_user" || lock.locked }
    /// Simple (or locked): the Snapshots window shows Recall and A/B only.
    var simple: Bool { mode == "simple" || lockedUser }
    /// The Installer pages show (Settings > Installer, At Power-On).
    var installerMode: Bool { mode == "installer" && !lockedUser }
    /// The switch may change the mode (no lock, no pinned mode).
    var switchable: Bool { choice?.switchable ?? true }
    /// The Installer pages, the profile and the PIN may change now.
    var installerOpen: Bool { lock.installerOpen }
    /// The hub holds this device's lock (one lock, one prompt).
    var hubHolds: Bool { lock.heldBy == "hub" }
    /// "Installer" in the bar: unlocked with a PIN set.
    var unlockedInstaller: Bool { lock.pinSet && lock.unlocked && mode == "installer" }

    // -- WP SW6 -- HUBADMIN

    /// The page edits the hub's copy.
    var hubCopy: Bool { available && target?.copy == "hub" }
    /// The page may change its copy: not before the target is read, not the
    /// hub's copy before it is read, and the hub's only as its Admin.
    var editable: Bool {
        guard available, let t = target else { return false }
        if t.copy == "hub" && hub == nil { return false }
        return t.editable
    }
    /// Which copy is edited, in the core's words (`installer.hub_copy`, ...).
    var copyLine: String {
        guard let key = target?.noticeKey, !key.isEmpty, key != "disconnected" else { return "" }
        return Self.line(key)
    }
    /// Why the page is read-only and what to do, in the core's words ("" when
    /// editable).
    var readOnlyLine: String {
        guard available, !editable, let key = target?.readOnlyKey, !key.isEmpty else { return "" }
        let n = noticeForKey(key: key, detail: "")
        let why = Self.line(key)
        return n.action.isEmpty ? why : "\(why) \(n.action)"
    }
    /// A PIN is set on the copy the page edits.
    var pinSet: Bool { hubCopy ? (hub?.pinSet ?? false) : lock.pinSet }
    /// The hub's pinned mode of each role it shows.
    var pinnedModes: [HubModePin] { hubCopy ? (hub?.pinnedModes ?? []) : [] }
    /// The recovery code's notice row.
    var recoveryKey: String { hubRecovery ? "installer_lock.hub_recovery" : "installer_lock.recovery" }

    /// The manifest's default for this platform class.
    static func defaultMode(_ m: ModeManifest) -> String {
        m.modes.first { $0.defaultOn.contains(platform) }?.id ?? "advanced"
    }

    /// The modes the switch lists, in the manifest's order (id, label).
    var modes: [ModeEntry] { manifest.modes }

    func label(ofMode id: String) -> String {
        if id == "locked_user" { return manifest.lockedUser.label }
        return manifest.modes.first { $0.id == id }?.label ?? id
    }

    /// The screens the locked-user state hides: what "Lock the settings" covers.
    var coverPages: [String] {
        let open = Set(manifest.lockedUser.screens)
        return manifest.screens.map(\.id).filter { !open.contains($0) }
    }

    /// The unlock prompt's line: the lock is on this console, the DSPi is
    /// open; or the hub's PIN unlocks it.
    var scopeLine: String {
        let key = lock.scopeNoticeKey.isEmpty ? (hubHolds ? "installer_lock.hub" : "installer_lock.scope")
                                              : lock.scopeNoticeKey
        return Self.reason(key)
    }

    /// The locked front page's line (`installer_lock.banner`).
    static var bannerLine: String {
        let r = reason("installer_lock.banner")
        return r.isEmpty ? line("installer_lock.banner", fallback: "Locked by the installer") : r
    }

    /// One field's range from the core's table.
    func range(_ key: String) -> LimitFieldRange? { ranges.first { $0.key == key } }

    // MARK: - Connection

    private var source: CoreStateSource? { vm?.transport.coreStateSource }

    private func connectionChanged(_ connected: Bool) {
        let isAvailable = connected && source != nil
        if available != isAvailable { available = isAvailable }
        chosen = Self.storedChoice(serial: vm?.selectedDevice?.serial)
        // -- WP SW6 -- another device, another copy: nothing editable until read
        target = nil
        hub = nil
        guard isAvailable else {
            if lock != Self.noLock { lock = Self.noLock }
            status = nil
            draft = LimitsDraft()
            withoutLimiter = []
            choice = nil
            return
        }
        listenHubEvents()
        nameFolder()
        refresh()
    }

    // -- WP SW6 -- The hub's `limits.changed`, `limits.held` and
    // `mode.changed` (HUBADMIN's `HubAdminListener`): the lock the core read
    // again at once, then the page from the hub.
    private func listenHubEvents() {
        guard let source = source, source !== listenedSource else { return }
        if let old = listenedSource { old.perform { $0.setHubAdminListener(listener: nil) } }
        listenedSource = source
        let events = hubEvents
        source.perform { $0.setHubAdminListener(listener: events) }
    }

    /// Main thread.
    fileprivate func hubEvent(_ event: HubAdminEvent) {
        guard available else { return }
        let open = target?.serial ?? vm?.selectedDevice?.serial ?? ""
        if !event.serial.isEmpty, !open.isEmpty, event.serial != open { return }
        if !event.lock.serial.isEmpty, lock != event.lock { lock = event.lock }
        if event.kind == "limits.held", !event.rows.isEmpty { post(event.rows.joined(separator: "\n"), .warning) }
        refresh()
    }

    /// Name the limits folder once per core.
    private func nameFolder() {
        guard let folder = folder, let source = source else { return }
        let key = ObjectIdentifier(source)
        guard !folderSet.contains(key) else { return }
        folderSet.insert(key)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let path = folder.path
        run({ try $0.limitsSetDir(dir: path) }) { [weak self] result in
            if case .failure(let e) = result {
                self?.folderSet.remove(key)
                self?.post(Self.line(for: e), .error)
            }
        }
    }

    /// Read the lock (the hub's too: `installerLockRefresh`), the profile,
    /// the S2 list and the mode again.
    func refresh() {
        guard available else { return }
        let chosen = self.chosen
        run({ core -> (LockState, LimitsStatus, String, Data, ModeChoice?, InstallerTarget, HubAdminState?) in
            let lock = (try? core.installerLockRefresh()) ?? core.installerLockState()
            let choice = try? core.modeResolve(platform: Self.platform, chosen: chosen)
            // -- WP SW6 -- which copy; the hub's when it is the hub's (then the
            // target again: it takes the role the hub just answered with)
            var target = core.installerTarget()
            var hub: HubAdminState?
            if target.copy == "hub" {
                hub = try? core.hubLimitsGet()
                target = core.installerTarget()
            }
            let document = target.copy == "hub" ? (hub?.profileDocument ?? "") : core.limitsProfileDocument()
            return (lock, core.limitsStatus(), document,
                    core.outputsWithoutLimiter(thresholdDb: Self.withoutLimiterAboveDb), choice, target, hub)
        }) { [weak self] result in
            guard let self = self, case .success(let (lock, status, document, without, choice, target, hub)) = result
            else { return }
            if self.target != target { self.target = target }   // -- WP SW6 --
            if self.hub != hub { self.hub = hub }
            self.apply(lock: lock, status: status, document: document, without: without, choice: choice)
        }
    }

    private func apply(lock: LockState, status: LimitsStatus?, document: String?, without: Data?, choice: ModeChoice?) {
        if self.lock != lock { self.lock = lock }
        if let status = status, self.status != status { self.status = status }
        if let document = document { draft = LimitsDraft(document: document) }
        if let without = without { withoutLimiter = without.map { Int($0) } }
        if let choice = choice, self.choice != choice { self.choice = choice }
    }

    /// The lock and the mode again, after a PIN call.
    private func resolveAfter(_ lock: LockState) {
        let chosen = self.chosen
        apply(lock: lock, status: nil, document: nil, without: nil, choice: nil)
        run({ core in (core.limitsProfileDocument(), try? core.modeResolve(platform: Self.platform, chosen: chosen)) }) {
            [weak self] result in
            guard case .success(let (document, choice)) = result else { return }
            self?.apply(lock: lock, status: nil, document: document, without: nil, choice: choice)
        }
    }

    // MARK: - Calls

    /// `body` on the core's queue, off main; `done` on main.
    private func run<T>(_ body: @escaping (DspiCore) throws -> T, done: @escaping (Result<T, DspiError>) -> Void) {
        guard let source = source else {
            done(.failure(.Disconnected))
            return
        }
        track(1)
        Self.work.async {
            let result = source.call(body)
            DispatchQueue.main.async {
                self.track(-1)
                done(result)
            }
        }
    }

    /// A write to the device (the profile's clamp, Protect all): through the
    /// write path, so the state and the journal are published after it.
    private func write<T>(_ body: @escaping (DspiCore) throws -> T, done: @escaping (Result<T, DspiError>) -> Void) {
        guard let source = source else {
            done(.failure(.Disconnected))
            return
        }
        track(1)
        Self.work.async {
            let result = source.writeAndWait(body)
            DispatchQueue.main.async {
                self.track(-1)
                done(result)
            }
        }
    }

    private func track(_ delta: Int) {
        running += delta
        let now = running > 0
        if busy != now { busy = now }
    }

    /// Show `text` as the notice line for a while.
    func post(_ text: String, _ kind: SnapshotsNotice.Kind = .info) {
        noticeClear?.cancel()
        notice = SnapshotsNotice(text: text, kind: kind)
        let clear = DispatchWorkItem { [weak self] in self?.notice = nil }
        noticeClear = clear
        DispatchQueue.main.asyncAfter(deadline: .now() + (kind == .info ? 8 : 12), execute: clear)
    }

    /// A PIN refusal in the core's words, with how long to wait.
    private func postPinError(_ error: DspiError) {
        var text = Self.line(for: error)
        if case .InstallerLock(let code, _) = error, code == "wait" {
            run({ $0.installerLockState() }) { [weak self] result in
                guard let self = self, case .success(let lock) = result else { return }
                self.apply(lock: lock, status: nil, document: nil, without: nil, choice: nil)
                if lock.retryAfterMs > 0 { text += " " + Self.retryLine(lock.retryAfterMs) }
                self.post(text, .warning)
            }
            return
        }
        post(text, .warning)
    }

    /// "Try again in 30 s." (whole seconds, rounded up).
    static func retryLine(_ ms: UInt64) -> String {
        let s = (ms + 999) / 1000
        return s >= 120 ? "Try again in \((s + 59) / 60) min." : "Try again in \(s) s."
    }

    // MARK: - The PIN and the lock

    /// Set the installer PIN (twice, the same).  The recovery code is shown
    /// once (`recoveryCode`).
    func setPin(_ pin: String, again: String) {
        guard pin == again else {
            post("The two PINs differ.", .warning)
            return
        }
        if hubCopy {   // -- WP SW6 -- the hub's PIN: an Admin needs no current one
            run({ try $0.hubPinSet(pin: pin) }) { [weak self] result in
                guard let self = self else { return }
                switch result {
                case .success(let made):
                    self.hubRecovery = true
                    self.recoveryCode = made.recoveryCode
                    self.hub = made.state
                    self.post("Installer PIN set on the hub.")
                case .failure(let e): self.postPinError(e)
                }
                self.refresh()
            }
            return
        }
        hubRecovery = false
        run({ try $0.installerPinSet(pin: pin) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let made):
                self.recoveryCode = made.recoveryCode
                self.post(made.state.locked ? "Installer PIN set and locked."
                          : "Installer PIN set. Unlocked on this console for this session.")
                self.resolveAfter(made.state)
            case .failure(let e): self.postPinError(e)
            }
        }
    }

    /// Unlock with the PIN or the recovery code (on the hub when it holds
    /// the lock).  `done(true)` when unlocked.
    func unlock(_ pin: String, done: @escaping (Bool) -> Void = { _ in }) {
        run({ try $0.installerUnlock(pin: pin) }) { [weak self] result in
            guard let self = self else { return done(false) }
            switch result {
            case .success(let lock):
                self.resolveAfter(lock)
                if lock.recovered { self.post("Unlocked with the recovery code. Set a new PIN.", .warning) }
                let then = self.afterUnlock
                self.afterUnlock = nil
                then?()
                done(true)
            case .failure(let e):
                self.postPinError(e)
                done(false)
            }
        }
    }

    /// Lock again now (until the PIN is entered).
    func lockNow() {
        run({ try $0.installerLock() }) { [weak self] result in
            switch result {
            case .success(let lock): self?.resolveAfter(lock)
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
        }
    }

    /// Remove the PIN: nothing is locked any more.
    func removePin() {
        if hubCopy {   // -- WP SW6 --
            run({ try $0.hubPinClear() }) { [weak self] result in
                switch result {
                case .success(let state):
                    self?.hub = state
                    self?.post("Installer PIN removed on the hub.")
                case .failure(let e): self?.post(Self.line(for: e), .warning)
                }
                self?.refresh()
            }
            return
        }
        run({ try $0.installerPinClear() }) { [weak self] result in
            switch result {
            case .success(let lock):
                self?.post("Installer PIN removed: nothing is locked.")
                self?.resolveAfter(lock)
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
        }
    }

    /// "Lock the settings": the lock covers every page the locked user does
    /// not see (on), or nothing (off: the PIN guards the Installer pages only).
    func setCovered(_ on: Bool) {
        let pages = on ? coverPages : []
        if hubCopy {   // -- WP SW6 -- what the hub's lock covers is its profile's lockedPages
            draft.setLockedPages(pages)
            sendProfile()
            return
        }
        run({ try $0.installerLockCover(pages: pages) }) { [weak self] result in
            switch result {
            case .success(let lock):
                // -- WP P1SW2F -- F1: the draft follows at once, so an edit
                // sent before the profile is read again keeps the cover.
                self?.draft.setLockedPages(lock.covers)
                self?.resolveAfter(lock)
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
        }
    }

    /// The unlock prompt over the main window; `then` runs once unlocked.
    func askUnlock(then: (() -> Void)? = nil) {
        afterUnlock = then
        unlockShown = true
    }

    // MARK: - The mode

    static let modeKeyAll = "uiMode.all"
    static func modeKey(serial: String) -> String { "uiMode.device.\(serial)" }

    /// The kept choice for `serial`: its own, else the one for every device.
    /// -- WP P1SW2F -- F7: `-UIMode simple|advanced` (UI tests,
    /// screenshots): the choice for this launch only, read from the launch
    /// arguments and never written; a choice made in the app replaces it.
    static var launchMode: String? = {
        let m = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)["UIMode"] as? String
        return (m?.isEmpty ?? true) ? nil : m
    }()

    static func storedChoice(serial: String?) -> String {
        if let m = launchMode { return m }
        let d = UserDefaults.standard
        if let serial = serial, let own = d.string(forKey: modeKey(serial: serial)), !own.isEmpty { return own }
        return d.string(forKey: modeKeyAll) ?? ""
    }

    /// Choose `mode` for the open device, or for every device on this
    /// console.  The Installer asks for the PIN when one is set and this
    /// console is not unlocked; with no PIN nothing asks.
    func choose(_ mode: String, everyDevice: Bool) {
        let keep = { [weak self] in
            guard let self = self else { return }
            let d = UserDefaults.standard
            if everyDevice {
                d.set(mode, forKey: Self.modeKeyAll)
                if let serial = self.vm?.selectedDevice?.serial { d.removeObject(forKey: Self.modeKey(serial: serial)) }
            } else if let serial = self.vm?.selectedDevice?.serial {
                d.set(mode, forKey: Self.modeKey(serial: serial))
            } else {
                d.set(mode, forKey: Self.modeKeyAll)
            }
            Self.launchMode = nil   // -- WP P1SW2F -- F7: the user's choice replaces the launch's
            self.chosen = mode
            self.resolve()
        }
        let needsPin = manifest.modes.first { $0.id == mode }?.needsPinWhenSet ?? false
        if needsPin && lock.pinSet && !lock.installerOpen {
            askUnlock(then: keep)
        } else {
            keep()
        }
    }

    /// `mode_resolve` again (no device: the manifest's default).
    func resolve() {
        guard available else {
            choice = nil
            return
        }
        let chosen = self.chosen
        run({ try $0.modeResolve(platform: Self.platform, chosen: chosen) }) { [weak self] result in
            if case .success(let c) = result, self?.choice != c { self?.choice = c }
        }
    }

    // -- WP SW6 --
    /// Pin `role`'s mode on the hub ("" for the user's choice): Admin only.
    func setModePin(role: String, mode: String) {
        guard hubCopy else { return }
        run({ try $0.hubModePin(role: role, mode: mode) }) { [weak self] result in
            switch result {
            case .success(let state): self?.hub = state
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
            self?.refresh()
        }
    }

    // MARK: - The limits profile

    /// Set a device-wide limit (nil: none).
    func setLimit(_ key: String, _ value: Float?) {
        draft.set(key, value.map { clamp(key, $0) })
        sendProfile()
    }

    /// Set an output's limit (nil: none).
    func setOutputLimit(_ output: Int, _ key: String, _ value: Float?) {
        draft.setOutput(output, key, value.map { clamp(key, $0) })
        sendProfile()
    }

    /// The installer's note or contact ("" none).
    func setText(_ key: String, _ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t != draft.text(key) else { return }
        draft.setText(key, t)
        sendProfile()
    }

    /// A value held to the core's range for `key`.
    func clamp(_ key: String, _ value: Float) -> Float {
        guard let r = range(key), value.isFinite else { return value }
        return min(max(value, r.min), r.max)
    }

    /// The profile to the core: applied at once, what was held reported.
    private func sendProfile() {
        // -- WP P1SW2F -- F1: what the lock covers is the lock's (the core's
        // state now), never a draft read before the last cover change.
        if lock.pinSet && !hubCopy { draft.setLockedPages(lock.covers) }
        let document = draft.document(name: "Limits profile")
        if hubCopy {   // -- WP SW6 -- set on the hub; what it held at once is shown
            run({ try $0.hubLimitsSet(document: document) }) { [weak self] result in
                guard let self = self else { return }
                switch result {
                case .success(let set):
                    self.hub = set.state
                    if !set.held.isEmpty { self.post(set.held.joined(separator: "\n"), .warning) }
                case .failure(let e):
                    self.post(Self.line(for: e), .warning)
                }
                self.reread()
                self.refresh()
            }
            return
        }
        write({ try $0.limitsApplyProfile(document: document) }) { [weak self] result in
            guard let self = self else { return }
            switch result {
            case .success(let applied):
                let lines = applied.lines.isEmpty ? applied.held : applied.lines
                if !lines.isEmpty { self.post(lines.joined(separator: "\n"), .warning) }
                if !applied.notes.isEmpty { self.post(applied.notes.joined(separator: " "), .warning) }
            case .failure(let e):
                self.post(Self.line(for: e), .warning)
            }
            self.reread()
            self.refresh()
        }
    }

    /// Remove every limit.  What the lock covers stays while a PIN is set.
    func clearLimits() {
        if hubCopy {   // -- WP SW6 -- the hub's profile goes (what its lock covers stays)
            if pinSet, !draft.lockedPages.isEmpty {
                draft = LimitsDraft(lockedPages: draft.lockedPages)
                sendProfile()
                return
            }
            run({ try $0.hubLimitsClear() }) { [weak self] result in
                switch result {
                case .success(let state):
                    self?.hub = state
                    self?.post("Limits removed on the hub.")
                case .failure(let e): self?.post(Self.line(for: e), .warning)
                }
                self?.refresh()
            }
            return
        }
        if lock.pinSet, !lock.covers.isEmpty {   // -- WP P1SW2F -- F1: the lock's
            draft = LimitsDraft(lockedPages: lock.covers)
            sendProfile()
            return
        }
        write({ try $0.limitsClear() }) { [weak self] result in
            switch result {
            case .success: self?.post("Limits removed.")
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
            self?.refresh()
        }
    }

    /// Every output's limiter on at -1 dBFS, stereo pairs linked: one undo
    /// step (the limiters stay on after it, the core's rule).
    func protectAllOutputs() {
        write({ try $0.protectAllOutputs(ceilingDbfs: Self.protectCeilingDb) }) { [weak self] result in
            switch result {
            case .success:
                self?.post("Every output's limiter is on at \(Int(Self.protectCeilingDb)) dBFS, pairs linked.")
            case .failure(let e): self?.post(Self.line(for: e), .warning)
            }
            self?.reread()
            self?.refresh()
        }
    }

    /// The view model shows what the device holds after a clamp.
    private func reread() {
        guard let vm = vm, let source = source else { return }
        Self.work.async { vm.applyCoreReread(source) }
    }

    /// "OUT3, OUT4" for the S2 line.
    func outputNames(_ outputs: [Int]) -> String {
        outputs.map { outputName($0) }.joined(separator: ", ")
    }

    func outputName(_ out: Int) -> String {
        guard let vm = vm else { return "Out \(out + 1)" }
        let idx = vm.chOut1 + out
        return idx < vm.channelNames.count ? vm.channelNames[idx] : "Out \(out + 1)"
    }

    var outputCount: Int { vm?.numOutputChannels ?? 0 }
}

// MARK: - The profile's document

/// The `limits` block of a limits document (limits-profiles.md), edited in
/// place as JSON: every field may be left out, and a field this build does
/// not know is kept as it was.
struct LimitsDraft: Equatable {
    private(set) var json: [String: Any] = [:]
    /// Bumped on every change, so SwiftUI sees one.
    private(set) var revision = 0

    init() {}

    init(lockedPages: [String]) {
        json = ["lockedPages": lockedPages]
    }

    /// -- WP P1SW2F -- What the lock covers (empty: the key goes).
    mutating func setLockedPages(_ pages: [String]) {
        if pages.isEmpty { json.removeValue(forKey: "lockedPages") } else { json["lockedPages"] = pages }
        revision += 1
    }

    /// From the core's document ("" none).
    init(document: String) {
        guard let object = (try? JSONSerialization.jsonObject(with: Data(document.utf8))) as? [String: Any],
              let limits = object["limits"] as? [String: Any] else { return }
        json = limits
        json.removeValue(forKey: "serial")   // the open device's: the core fills it
    }

    static func == (a: LimitsDraft, b: LimitsDraft) -> Bool {
        a.revision == b.revision && NSDictionary(dictionary: a.json).isEqual(to: b.json)
    }

    var lockedPages: [String] { json["lockedPages"] as? [String] ?? [] }
    var isEmpty: Bool { json.keys.allSatisfy { $0 == "lockedPages" } }

    func value(_ key: String) -> Float? { (json[key] as? NSNumber)?.floatValue }
    func text(_ key: String) -> String { json[key] as? String ?? "" }

    func outputValue(_ output: Int, _ key: String) -> Float? {
        (outputRow(output)?[key] as? NSNumber)?.floatValue
    }

    private func outputRow(_ output: Int) -> [String: Any]? {
        (json["outputs"] as? [[String: Any]])?.last { ($0["output"] as? NSNumber)?.intValue == output }
    }

    mutating func set(_ key: String, _ value: Float?) {
        if let v = value, v.isFinite { json[key] = Double(v) } else { json.removeValue(forKey: key) }
        revision += 1
    }

    mutating func setText(_ key: String, _ text: String) {
        if text.isEmpty { json.removeValue(forKey: key) } else { json[key] = text }
        revision += 1
    }

    mutating func setOutput(_ output: Int, _ key: String, _ value: Float?) {
        var rows = json["outputs"] as? [[String: Any]] ?? []
        let at = rows.lastIndex { ($0["output"] as? NSNumber)?.intValue == output }
        var row = at.map { rows[$0] } ?? ["output": output]
        if let v = value, v.isFinite { row[key] = Double(v) } else { row.removeValue(forKey: key) }
        // The slope only means something with the corner.
        if key == "highpassMinHz", value == nil { row.removeValue(forKey: "highpassMinSlopeDb") }
        let empty = row.count <= 1
        if let at = at {
            if empty { rows.remove(at: at) } else { rows[at] = row }
        } else if !empty {
            rows.append(row)
        }
        if rows.isEmpty { json.removeValue(forKey: "outputs") } else { json["outputs"] = rows }
        revision += 1
    }

    /// The whole document for `limits_apply_profile`.
    func document(name: String) -> String {
        let doc: [String: Any] = ["schemaVersion": 1,
                                  "meta": ["kind": "limits", "name": name],
                                  "limits": json]
        guard let data = try? JSONSerialization.data(withJSONObject: doc, options: [.sortedKeys]) else { return "" }
        return String(decoding: data, as: UTF8.self)
    }
}

// -- WP SW6 --
/// The hub's admin events (HUBADMIN), called on the Link session's
/// supervisor with no core lock held: only hops to main.
private final class HubEvents: HubAdminListener, @unchecked Sendable {
    private weak var installer: CoreInstaller?
    init(_ installer: CoreInstaller) { self.installer = installer }
    func onHubAdminEvent(event: HubAdminEvent) {
        DispatchQueue.main.async { [weak self] in self?.installer?.hubEvent(event) }
    }
}
