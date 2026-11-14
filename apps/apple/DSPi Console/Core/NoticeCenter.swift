//
//  NoticeCenter.swift
//  DSPi Console
//
//  WP P1SW3 (PRO1 U5).  Notices that need no answer, as Qt's `NoticeCenter`
//  (QP18, P1QT2): a failed export, a refused save, a finished import.  They
//  show in the window's notice host (`NoticeHost`), never as a modal alert.
//  An info notice goes by itself after a while; a warning or an error stays
//  until it is dismissed.  A question that needs an answer stays a modal.
//
//  The words of a failure are the core's (P1NT, `assets/notices/notices.json`
//  through `noticeForKey` / `noticeFor`): a notice is built from a row by its
//  key, and the Swift words of what happened go in the row's detail line.
//  There is no success row: a success is the change itself, or a short result
//  line (`ConsoleNotice.result`), never a "Success" modal.
//
//  One host per window: the main window's is `NoticeCenter.main` (the menus'
//  file, preset and AutoEQ actions post there); the Snapshots and Settings
//  windows keep their own notice lines (P1SW1, P1SW2).
//

import Foundation
import SwiftUI
import dspi_core

/// One notice: the core's row (title, reason, action) and its detail, or a
/// result line with no row (`key` empty).
struct ConsoleNotice: Equatable {
    enum Kind: Equatable { case info, warning, error }

    /// The core's row key (`disconnected`, `io`, ...); "" for a result line.
    let key: String
    let kind: Kind
    let title: String
    let reason: String
    let action: String
    /// What happened, in this app's words (may be empty).
    let detail: String

    /// The row in one line, "Title: reason" (no detail).
    var sentence: String { reason.isEmpty ? title : "\(title): \(reason)" }

    /// The notice in one line: "Title: reason", then the detail.
    var line: String {
        var s = reason.isEmpty ? title : "\(title): \(reason)"
        if !detail.isEmpty { s += " (\(detail))" }
        return s
    }

    // MARK: - From the core's table

    /// The row for `key` with `detail`.  A key the table lacks falls back to
    /// its family's row (`installer_lock.x` to `installer_lock`), then to the
    /// core's `unknown` row.
    static func row(_ key: String, detail: String = "") -> ConsoleNotice {
        var n = noticeForKey(key: key, detail: detail)
        var family = key
        while n.key != family, let dot = family.lastIndex(of: ".") {
            family = String(family[..<dot])
            n = noticeForKey(key: family, detail: detail)
        }
        return ConsoleNotice(n)
    }

    /// The core's notice for `error`.
    static func of(_ error: DspiError, detail: String? = nil) -> ConsoleNotice {
        let n = noticeFor(error: error)
        return ConsoleNotice(key: n.key, kind: Kind(n.kind), title: n.title, reason: n.reason,
                             action: n.action, detail: detail ?? n.detail)
    }

    /// An error of any type: the core's own notice for a `DspiError`, the row
    /// `fallbackKey` with the error's words otherwise.
    static func of(_ error: Error, fallbackKey: String, detail: String? = nil) -> ConsoleNotice {
        if let e = error as? DspiError { return of(e, detail: detail) }
        return row(fallbackKey, detail: detail ?? error.localizedDescription)
    }

    /// A result with no row: what an action did (info), or a report that
    /// needs reading (`kind` warning).  Never a "Success" heading.
    static func result(_ text: String, kind: Kind = .info) -> ConsoleNotice {
        ConsoleNotice(key: "", kind: kind, title: text, reason: "", action: "", detail: "")
    }

    /// A longer report under a heading: what an import did (info), or what
    /// it could not do (warning, stays until read).
    static func report(_ title: String, _ text: String, kind: Kind = .info) -> ConsoleNotice {
        ConsoleNotice(key: "", kind: kind, title: title, reason: text, action: "", detail: "")
    }

    init(key: String, kind: Kind, title: String, reason: String, action: String, detail: String) {
        self.key = key
        self.kind = kind
        self.title = title
        self.reason = reason
        self.action = action
        self.detail = detail
    }

    init(_ n: Notice) {
        self.init(key: n.key, kind: Kind(n.kind), title: n.title, reason: n.reason, action: n.action,
                  detail: n.detail)
    }
}

extension ConsoleNotice.Kind {
    init(_ kind: NoticeKind) {
        switch kind {
        case .info: self = .info
        case .warning: self = .warning
        case .error: self = .error
        }
    }
}

// MARK: - A status byte with the core's error

/// -- WP P1SW3F -- F1: a preset or flash call's status byte (`PRESET_*`,
/// `FLASH_*`) and, through the core, the error behind a failure, so the
/// notice is the core's own row for it.  `error` is nil on success and on
/// the raw (non-core) path.
struct CoreOutcome: Equatable {
    let status: UInt8
    let error: DspiError?

    init(_ status: UInt8, _ error: DspiError? = nil) {
        self.status = status
        self.error = error
    }
}

// MARK: - The app's failures, as the core's rows

/// What the converted alerts say (P1SW3): each failure is a core row, with
/// what failed in the detail line.  Pure, so the tests read them.
enum AppNotices {
    /// No device open.
    static var notConnected: ConsoleNotice { .row("disconnected") }
    /// The device has not identified itself yet.
    static var stillReading: ConsoleNotice { .row("config.not_read") }
    /// Firmware in another wire format: shown, not written (SW8).
    static var readOnly: ConsoleNotice { .row("read_only") }
    /// Another console holds the hub's device lock.
    static func locked(_ what: String = "") -> ConsoleNotice { .row("locked", detail: what) }
    /// A file that could not be read, written or understood.
    static func file(_ what: String) -> ConsoleNotice { .row("io", detail: what) }
    static func file(_ error: Error) -> ConsoleNotice { .row("io", detail: error.localizedDescription) }
    /// A configuration file this version does not read.
    static func configUnreadable(_ error: Error) -> ConsoleNotice {
        .of(error, fallbackKey: "config.unreadable")
    }
    /// The device did not do what was asked.
    static func transfer(_ what: String) -> ConsoleNotice { .row("transfer", detail: what) }

    /// A preset call's failure status (`PRESET_*`): the lock, or a transfer.
    /// -- WP P1SW3F -- F1: with the core's error, the core's row for it (the
    /// installer's lock, read-only firmware, a limit, a missing confirmation,
    /// the hub's denial or lock), `what` as its detail.
    static func preset(_ status: UInt8, _ what: String, error: DspiError? = nil) -> ConsoleNotice {
        if let error, !isRefusalByte(error) { return .of(error, detail: what) }
        return status == PRESET_STATUS_LOCKED ? locked(what) : transfer("\(what) (error \(status)).")
    }

    static func preset(_ outcome: CoreOutcome, _ what: String) -> ConsoleNotice {
        preset(outcome.status, what, error: outcome.error)
    }

    /// The device's own refusal byte: the status says more than the core's
    /// generic `refused` row.
    private static func isRefusalByte(_ error: DspiError) -> Bool {
        if case .Refused = error { return true }
        return false
    }

    /// A preset load that failed (the "Load Failed" alert).
    static func presetLoad(_ status: UInt8, error: DspiError? = nil) -> ConsoleNotice {
        status == PRESET_ERR_CRC ? transfer("Preset data is corrupted.")
                                 : preset(status, "Failed to load preset", error: error)
    }

    static func presetLoad(_ outcome: CoreOutcome) -> ConsoleNotice { presetLoad(outcome.status, error: outcome.error) }

    /// Revert to Saved's outcome (`FLASH_*`): a result line, or a row.
    static func revert(_ status: UInt8, error: DspiError? = nil) -> ConsoleNotice {
        switch status {
        case FLASH_OK: return .result("Parameters reverted")
        case FLASH_ERR_NO_DATA: return .result("No saved parameters found. The device is using factory defaults.")
        case FLASH_ERR_CRC: return transfer("Saved data is corrupted.")
        default:
            if let error, !isRefusalByte(error) { return .of(error, detail: "Failed to load parameters.") }
            return transfer("Failed to load parameters.")
        }
    }

    static func revert(_ outcome: CoreOutcome) -> ConsoleNotice { revert(outcome.status, error: outcome.error) }

    /// Factory Reset's outcome.
    static func factoryReset(_ status: UInt8, error: DspiError? = nil) -> ConsoleNotice {
        if status == FLASH_OK { return .result("Factory reset complete") }
        if let error, !isRefusalByte(error) { return .of(error, detail: "Failed to reset parameters.") }
        return transfer("Failed to reset parameters.")
    }

    static func factoryReset(_ outcome: CoreOutcome) -> ConsoleNotice {
        factoryReset(outcome.status, error: outcome.error)
    }

    /// Save Output Configuration's failure: the core's row with its error,
    /// else the hub's lock or a device that did not acknowledge.
    static func outputConfig(_ failure: LinkStatus, error: DspiError?) -> ConsoleNotice {
        let what = "Output configuration not saved."
        if let error, !isRefusalByte(error) { return .of(error, detail: what) }
        return failure == .locked ? locked(what)
                                  : transfer("Output configuration not saved: the device did not acknowledge the request.")
    }

    /// An AutoEQ database update that failed.
    static func autoEQ(_ error: Error) -> ConsoleNotice { .of(error, fallbackKey: "autoeq") }
}

// MARK: - The center

/// A window's notices, one at a time.  Main thread; `post` from any thread.
final class NoticeCenter: ObservableObject {
    /// The main window's notices (the menus post here).
    static let main = NoticeCenter()

    /// How long an info notice stays, seconds.
    static let infoSeconds: TimeInterval = 8
    /// Older notices beyond this many waiting are dropped.
    static let maxWaiting = 5

    /// The notice on show, nil with none.
    @Published private(set) var current: ConsoleNotice?
    /// Notices waiting behind it.
    @Published private(set) var waiting: [ConsoleNotice] = []
    /// Counts every notice posted (tests).
    private(set) var serial = 0
    /// The last notice posted, shown or not (tests).
    private(set) var last: ConsoleNotice?

    private var clearWork: DispatchWorkItem?
    private let infoSeconds: TimeInterval
    /// -- WP P1SW3F -- F4: hosts on screen.  An info notice's time runs only
    /// while one shows it; with none it waits.
    private(set) var visibleHosts = 0

    init(infoSeconds: TimeInterval = NoticeCenter.infoSeconds) {
        self.infoSeconds = infoSeconds
    }

    /// Show `notice`, now or after the ones on show.  The same notice as the
    /// one on show is not queued again.
    func post(_ notice: ConsoleNotice) {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { self.post(notice) }
            return
        }
        serial += 1
        last = notice
        if current == nil {
            show(notice)
        } else if current == notice {
            // -- WP P1SW3F -- F5: a repeat restarts its time, as in Qt
            startTimer()
        } else if waiting.last != notice {
            waiting.append(notice)
            if waiting.count > Self.maxWaiting { waiting.removeFirst(waiting.count - Self.maxWaiting) }
        }
    }

    /// The core's row for `key`, with `detail`.
    func post(key: String, detail: String = "") { post(.row(key, detail: detail)) }

    /// A result line with no row (info).
    func post(result text: String) { post(.result(text)) }

    /// Put the notice on show away; the next one waiting follows.
    func dismiss() {
        clearWork?.cancel()
        clearWork = nil
        if waiting.isEmpty {
            current = nil
        } else {
            show(waiting.removeFirst())
        }
    }

    /// Everything away.
    func clear() {
        clearWork?.cancel()
        clearWork = nil
        waiting = []
        current = nil
    }

    private func show(_ notice: ConsoleNotice) {
        current = notice
        startTimer()
    }

    /// An info notice's time from now, while a host shows it.
    private func startTimer() {
        clearWork?.cancel()
        clearWork = nil
        guard current?.kind == .info, visibleHosts > 0 else { return }
        let work = DispatchWorkItem { [weak self] in self?.dismiss() }
        clearWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + infoSeconds, execute: work)
    }

    /// -- WP P1SW3F -- F4: a host came on screen (its `onAppear`).
    func hostAppeared() {
        visibleHosts += 1
        if visibleHosts == 1 { startTimer() }
    }

    /// A host went (its `onDisappear`); with none left the time stops.
    func hostDisappeared() {
        visibleHosts = max(0, visibleHosts - 1)
        if visibleHosts == 0 {
            clearWork?.cancel()
            clearWork = nil
        }
    }
}

// MARK: - The host

/// A window's notice host: the notice on show, over the bottom of the
/// window, with a dismiss button.  Draws nothing with no notice.
struct NoticeHost: View {
    @ObservedObject var center: NoticeCenter

    var body: some View {
        // -- WP P1SW3F -- F4: always in the hierarchy (zero height with no
        // notice), so the centre knows a host is on screen.
        VStack(spacing: 0) { notice }
            .onAppear { center.hostAppeared() }
            .onDisappear { center.hostDisappeared() }
    }

    @ViewBuilder private var notice: some View {
        if let n = center.current {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: Self.icon(n.kind))
                    .foregroundColor(Self.tint(n.kind))
                    .padding(.top, 1)
                VStack(alignment: .leading, spacing: 2) {
                    Text(n.title)
                        .font(.callout.weight(.semibold))
                        .accessibilityIdentifier("noticeTitle")
                    if !n.reason.isEmpty {
                        Text(n.reason).font(.callout)
                    }
                    if !n.detail.isEmpty {
                        Text(n.detail).font(.caption).foregroundColor(.secondary)
                            .accessibilityIdentifier("noticeDetail")
                    }
                    if !n.action.isEmpty {
                        Text(n.action).font(.caption).foregroundColor(.secondary)
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                Spacer(minLength: 0)
                if !center.waiting.isEmpty {
                    Text("+\(center.waiting.count)")
                        .font(.caption.monospacedDigit())
                        .foregroundColor(.secondary)
                }
                Button { center.dismiss() } label: {
                    Image(systemName: "xmark").font(.caption.weight(.bold))
                }
                .buttonStyle(.borderless)
                .help("Dismiss")
                .accessibilityLabel("Dismiss")
                .accessibilityIdentifier("noticeDismiss")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Self.tint(n.kind).opacity(0.12))
            .background(.regularMaterial)
            .overlay(Divider(), alignment: .top)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("noticeHost")
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    static func icon(_ kind: ConsoleNotice.Kind) -> String {
        switch kind {
        case .info: return "info.circle.fill"
        case .warning: return "exclamationmark.triangle.fill"
        case .error: return "xmark.octagon.fill"
        }
    }

    static func tint(_ kind: ConsoleNotice.Kind) -> Color {
        switch kind {
        case .info: return .accentColor
        case .warning: return .orange
        case .error: return .red
        }
    }
}
