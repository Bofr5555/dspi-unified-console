//
//  CoreUndo.swift
//  DSPi Console
//
//  WP SWMAC1.  Undo and Redo through the Rust core's change journal (P0W,
//  QTGAP1), as the Qt app's Edit menu (`HistoryController`): Edit > Undo
//  (Cmd-Z) and Redo (Shift-Cmd-Z) on the Mac, an Undo and a Redo button on
//  iOS.  Slider and graph drags are marked with the core's `setDragging`, so a
//  drag is one step however long it lasts.
//
//  Only while the device is on the core: behind `-CoreTransport YES`
//  (`CoreBackedTransport`: the core's demo device, or on the Mac the USB
//  device through the hub's session 1); otherwise there is no journal.  The journal undoes the core's typed setters;
//  the domains still send raw SETs until SW9-SW14 move them to the typed API,
//  and a raw SET is history only (`raw_set`: nobody knows what it moved), so
//  until then most edits cannot be undone and Undo stays disabled.
//  -- WP SW9 -- Volume, EQ, crossover, outputs and the matrix are on the
//  typed setters now (Commands.swift), so those edits are undone.
//

import Foundation
import Combine
import SwiftUI
import dspi_core

/// Whether the core's journal has a change to undo and one to redo.
struct CoreJournalState: Equatable {
    var canUndo = false
    var canRedo = false
}

/// One `undo()` or `redo()` and what the core holds after it.
struct CoreJournalStep {
    let result: Result<HistoryMove, DspiError>
    let state: DspState
    let presetChanges: [String]
    let journal: CoreJournalState
}

/// The Undo and Redo commands of one view model.  Main thread only.
final class CoreUndo: ObservableObject {
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false
    /// What the last step has to say when it did not simply move: a change
    /// stepped over, another writer's, an error.  Cleared after a while.
    @Published private(set) var notice: String?

    private weak var mirror: CoreMirror?
    /// A step on its way: a second press waits for it, not for its own.
    private var stepping = false
    private var noticeClear: DispatchWorkItem?

    init(mirror: CoreMirror) {
        self.mirror = mirror
    }

    /// The journal after a change, a step or a new connection.
    func show(_ journal: CoreJournalState) {
        if canUndo != journal.canUndo { canUndo = journal.canUndo }
        if canRedo != journal.canRedo { canRedo = journal.canRedo }
    }

    func undo() { step(redo: false) }
    func redo() { step(redo: true) }

    private func step(redo: Bool) {
        guard !stepping, redo ? canRedo : canUndo, let mirror = mirror else { return }
        stepping = mirror.stepJournal(redo: redo) { [weak self] result in
            self?.stepping = false
            if let result = result { self?.post(Self.message(for: result)) }
        }
    }

    /// The words for a step's result, nil when it simply moved.
    static func message(for result: Result<HistoryMove, DspiError>) -> String? {
        switch result {
        case .success(let move):
            if let note = move.note, !note.isEmpty { return note }
            switch move.skipped.count {
            case 0: return nil
            case 1: return "Stepped over a change that cannot be undone."
            default: return "Stepped over \(move.skipped.count) changes that cannot be undone."
            }
        case .failure(let error):
            switch error {
            case .Transfer(let msg), .InvalidArgument(let msg), .Link(_, let msg): return msg
            case .Disconnected: return "No device."
            case .ReadOnly, .Locked: return nil   // their own notices say it
            default: return String(describing: error)
            }
        }
    }

    private func post(_ text: String?) {
        noticeClear?.cancel()
        notice = text
        guard text != nil else { return }
        let clear = DispatchWorkItem { [weak self] in self?.notice = nil }
        noticeClear = clear
        DispatchQueue.main.asyncAfter(deadline: .now() + 4, execute: clear)
    }

    // MARK: - Drags

    /// A slider or graph drag of the app's console started (true) or ended
    /// (false).  The end is queued one main-loop turn later, behind the
    /// commit write the same callback sends, so the commit joins the drag.
    static func drag(_ dragging: Bool) {
        let mark: () -> Void = { AppState.shared.viewModel.transport.coreStateSource?.setDragging(dragging) }
        if dragging { mark() } else { DispatchQueue.main.async(execute: mark) }
    }
}

/// What the last undo or redo had to say (`CoreUndo.notice`), over the
/// bottom of the main window while it lasts.
struct UndoNotice: View {
    @ObservedObject var undo: CoreUndo

    var body: some View {
        if let text = undo.notice {
            Text(text)
                .font(.callout)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(.regularMaterial, in: Capsule())
                .padding(.bottom, 12)
                .transition(.opacity)
                .accessibilityIdentifier("undoNotice")
        }
    }
}
