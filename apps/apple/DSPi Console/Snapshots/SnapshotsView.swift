//
//  SnapshotsView.swift
//  DSPi Console
//
//  WP P1SW1.  The Snapshots window (macOS: Tools > Snapshots...; iOS: a
//  sheet at half height from the Tools menu), as Qt's SnapshotsWindow and
//  Android's Snapshots screen: capture, list, rename, note, recall, delete,
//  export and import, and the A/B compare with the diff between A and B.
//  The logic is CoreSnapshots'; this only draws it.
//
//  Simple shows the snapshots read-mostly: Recall and A/B (coordinator,
//  2026-11-13).  Advanced adds capture, rename, note, delete, import,
//  export, the rollback choice for a recall on USB and the safe apply's
//  timeout.  The app has no mode switch yet (P1SW2's), so the window keeps
//  its own: Advanced on the Mac, Simple first on iOS.
//

import SwiftUI
import dspi_core
#if os(macOS)
import AppKit
import UniformTypeIdentifiers
#else
import UIKit
import UniformTypeIdentifiers
#endif

struct SnapshotsView: View {
    @ObservedObject var snaps: CoreSnapshots
    /// Advanced (true) or Simple; remembered on this console.
    @AppStorage("snapshots.advanced") private var advanced: Bool = SnapshotsView.advancedByDefault
    @State private var captureName = ""
    @State private var aId = ""
    @State private var bId = ""
    @State private var renaming: SnapshotInfo?
    @State private var renameText = ""
    @State private var noting: SnapshotInfo?
    @State private var noteText = ""
    @State private var deleting: SnapshotInfo?
    @State private var showDiff = true

    #if os(macOS)
    static let advancedByDefault = true
    #else
    static let advancedByDefault = false
    #endif

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            if !snaps.available {
                Text("Snapshots need a device or a file on the core. Connect one first.")
                    .foregroundColor(.secondary)
                    .accessibilityIdentifier("snapshotsUnavailable")
            } else {
                if advanced { captureRow }
                compareSection
                listSection
                if advanced { optionsSection }
            }
            if let notice = snaps.notice {
                SnapshotsNoticeLine(notice: notice)
            }
        }
        .padding(16)
        .frame(minWidth: 320, idealWidth: 520, maxWidth: .infinity, alignment: .topLeading)
        .alert("Rename Snapshot", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $renameText)
            Button("Rename") {
                if let s = renaming { snaps.rename(s, to: String(renameText.prefix(CoreSnapshots.nameMax))) }
                renaming = nil
            }
            Button("Cancel", role: .cancel) { renaming = nil }
        }
        .alert("Snapshot Note", isPresented: Binding(get: { noting != nil }, set: { if !$0 { noting = nil } })) {
            TextField("Note", text: $noteText)
            Button("Save") {
                if let s = noting { snaps.setNote(s, String(noteText.prefix(CoreSnapshots.noteMax))) }
                noting = nil
            }
            Button("Cancel", role: .cancel) { noting = nil }
        } message: {
            Text(noting?.name ?? "")
        }
        .confirmationDialog(deleting.map { "Delete \u{201C}\($0.name)\u{201D}?" } ?? "",
                            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                            titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                if let s = deleting { snaps.delete(s) }
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        } message: {
            Text("The snapshot is removed from this console. The device keeps its settings.")
        }
        .onAppear { snaps.reload() }
        .accessibilityElement(children: .contain)
        .onChange(of: snaps.snapshots) { _, list in
            let ids = Set(list.map(\.id))
            if !aId.isEmpty, !ids.contains(aId) { aId = "" }
            if !bId.isEmpty, !ids.contains(bId) { bId = "" }
        }
        .accessibilityIdentifier("snapshotsView")
    }

    // MARK: Parts

    private var header: some View {
        HStack {
            Text("Snapshots").font(.title3.weight(.semibold))
            Spacer()
            Picker("Mode", selection: $advanced) {
                Text("Simple").tag(false)
                Text("Advanced").tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 200)
            .accessibilityIdentifier("snapshotsMode")
        }
    }

    private var captureRow: some View {
        HStack {
            TextField("Name (optional)", text: $captureName)
                .textFieldStyle(.roundedBorder)
                .onSubmit(capture)
                .accessibilityIdentifier("snapshotName")
            Button("Capture", action: capture)
                .disabled(snaps.busy)
                .accessibilityIdentifier("snapshotCapture")
            Button("Import...") { SnapshotFiles.importSnapshot(into: snaps) }
                .disabled(snaps.busy)
        }
    }

    private func capture() {
        snaps.capture(name: String(captureName.prefix(CoreSnapshots.nameMax)))
        captureName = ""
    }

    private var compareSection: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                if snaps.compare.active {
                    HStack(spacing: 10) {
                        Text("A: \(snaps.compare.aName)").fontWeight(snaps.compare.side == .a ? .bold : .regular)
                        Text("B: \(snaps.compare.bName)").fontWeight(snaps.compare.side == .b ? .bold : .regular)
                        Spacer()
                        Text("On \(snaps.compare.side == .b ? "B" : "A")")
                            .font(.callout.monospaced())
                            .accessibilityIdentifier("abSide")
                    }
                    HStack {
                        // The B key: CoreSnapshots' key monitor on the Mac
                        // (never while typing or asked something).
                        Button("Switch A/B") { snaps.toggle() }
                            .disabled(!snaps.canToggle)
                            .help("Key: B")
                            .accessibilityIdentifier("abToggle")
                        Button("Keep A") { snaps.keep(side: .a) }.disabled(snaps.busy)
                        Button("Keep B") { snaps.keep(side: .b) }.disabled(snaps.busy)
                    }
                    if snaps.guardWaiting {
                        Text(CoreSnapshots.finishFirst).font(.caption).foregroundColor(.orange)
                    }
                    if !snaps.abRows.isEmpty {
                        DisclosureGroup("What differs (\(snaps.abRows.count))", isExpanded: $showDiff) {
                            DiffRowsList(rows: snaps.abRows)
                        }
                    }
                } else {
                    HStack {
                        Picker("A", selection: $aId) {
                            Text("The device now").tag("")
                            ForEach(snaps.snapshots, id: \.id) { s in Text(s.name).tag(s.id) }
                        }
                        Picker("B", selection: $bId) {
                            Text("Choose...").tag("")
                            ForEach(snaps.snapshots, id: \.id) { s in Text(s.name).tag(s.id) }
                        }
                    }
                    Button("Compare") { snaps.startCompare(aId: aId, bId: bId) }
                        .disabled(bId.isEmpty || aId == bId || snaps.busy || snaps.guardWaiting)
                        .accessibilityIdentifier("abStart")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Text("A/B compare")
        }
    }

    private var listSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            if snaps.snapshots.isEmpty {
                Text(advanced ? "No snapshots of this device yet. Capture one above."
                              : "No snapshots of this device yet.")
                    .foregroundColor(.secondary)
            }
            ForEach(snaps.snapshots, id: \.id) { s in
                SnapshotRow(snapshot: s, advanced: advanced, busy: snaps.busy,
                            recallTitle: (snaps.guardDefault || snaps.rollback) ? "Recall with rollback" : "Recall",
                            recall: { snaps.recall(s) },
                            rename: { renameText = s.name; renaming = s },
                            note: { noteText = s.note; noting = s },
                            export: { SnapshotFiles.export(s, from: snaps) },
                            delete: { deleting = s })
            }
        }
    }

    private var optionsSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Toggle("Recall with rollback (Keep or Revert)", isOn: Binding(
                get: { snaps.guardDefault || snaps.rollback }, set: { snaps.rollback = $0 }))
                .disabled(snaps.guardDefault)
                .accessibilityIdentifier("snapshotRollback")
            if snaps.guardDefault {
                Text("Always over DSPi Link.").font(.caption).foregroundColor(.secondary)
            }
            Stepper(value: $snaps.timeoutS, in: CoreSnapshots.timeoutRange, step: 5) {
                Text("Safe apply waits \(snaps.timeoutS) s for Keep")
            }
            .accessibilityIdentifier("snapshotTimeout")
        }
    }
}

/// One snapshot: its name, when, its note and its actions.
private struct SnapshotRow: View {
    let snapshot: SnapshotInfo
    let advanced: Bool
    let busy: Bool
    let recallTitle: String
    let recall: () -> Void
    let rename: () -> Void
    let note: () -> Void
    let export: () -> Void
    let delete: () -> Void

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                Text(snapshot.name).fontWeight(.semibold)
                Text(SnapshotRow.saved(snapshot.savedMs)).font(.caption).foregroundColor(.secondary)
                if !snapshot.note.isEmpty {
                    Text(snapshot.note).font(.caption).foregroundColor(.secondary).lineLimit(3)
                }
            }
            Spacer()
            Button(recallTitle, action: recall)
                .disabled(busy)
                .accessibilityIdentifier("snapshotRecall")
            if advanced {
                Menu {
                    Button("Rename...", action: rename)
                    Button("Note...", action: note)
                    Button("Export...", action: export)
                    Divider()
                    Button("Delete...", role: .destructive, action: delete)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuIndicator(.hidden)
                .fixedSize()
                .disabled(busy)
                .accessibilityLabel("More")
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
    }

    static func saved(_ ms: Int64) -> String {
        guard ms > 0 else { return "" }
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }
}

/// P1DF's rows: "Section · item" over "before → after".
struct DiffRowsList: View {
    let rows: [SnapshotDiffRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(rows) { r in
                VStack(alignment: .leading, spacing: 1) {
                    Text(r.place).font(.callout)
                    Text("\(r.before.isEmpty ? "–" : r.before) → \(r.after.isEmpty ? "–" : r.after)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The notice line under a window's content (a result, never a modal).
struct SnapshotsNoticeLine: View {
    let notice: SnapshotsNotice

    var body: some View {
        Text(notice.text)
            .font(.callout)
            .foregroundColor(notice.kind == .error ? .red : (notice.kind == .warning ? .orange : .primary))
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("snapshotsNotice")
    }
}

// MARK: - Over the main window

/// The safe apply's countdown bar with Keep and Revert, and the end's
/// sentence after one this console saw waiting; plus the A/B chip while a
/// compare runs and the notice line.  Over the bottom of the main window.
struct SnapshotsOverlay: View {
    @ObservedObject var snaps: CoreSnapshots

    var body: some View {
        VStack(spacing: 6) {
            if snaps.barShown { SafeApplyBar(snaps: snaps) }
            if snaps.compare.active { ABChip(snaps: snaps) }
            if let notice = snaps.notice, !snaps.barShown {
                SnapshotsNoticeLine(notice: notice)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.regularMaterial, in: Capsule())
            }
        }
        .padding(.bottom, 12)
        .padding(.horizontal, 12)
    }
}

struct SafeApplyBar: View {
    @ObservedObject var snaps: CoreSnapshots

    var body: some View {
        let (first, second) = snaps.barLines
        let g = snaps.guardState
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(first).font(.callout.weight(.semibold))
                if !second.isEmpty { Text(second).font(.caption) }
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("safeApplyText")
            Spacer(minLength: 8)
            if g.phase == .waiting {
                ProgressView(value: Double(g.remainingMs), total: Double(max(g.timeoutMs, 1)))
                    .frame(width: 80)
            }
            if snaps.canRevert {
                Button("Revert") { snaps.revertChange() }
                    .accessibilityIdentifier("safeApplyRevert")
            }
            if g.phase == .waiting {
                Button("Keep") { snaps.keepChange() }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("safeApplyKeep")
            } else {
                Button("OK") { snaps.dismissEnd() }
                    .accessibilityIdentifier("safeApplyOK")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.thickMaterial, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.orange.opacity(0.6)))
        .frame(maxWidth: 640)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("safeApplyBar")
    }
}

/// "A/B: on A" with the switch, while a compare runs.
struct ABChip: View {
    @ObservedObject var snaps: CoreSnapshots

    var body: some View {
        HStack(spacing: 8) {
            Text("A/B: on \(snaps.compare.side == .b ? "B" : "A")").font(.callout.monospaced())
            Button("Switch") { snaps.toggle() }
                .disabled(!snaps.canToggle)
                .accessibilityIdentifier("abChipToggle")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(.regularMaterial, in: Capsule())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("abChip")
    }
}

/// "File: <name>" while the connection is a document opened offline.
struct OfflineFileBadge: View {
    @ObservedObject var snaps: CoreSnapshots

    var body: some View {
        if let doc = snaps.offline {
            HStack(spacing: 8) {
                Image(systemName: "doc.text")
                Text(doc.badge).font(.callout)
                Text("No device: changes go to the file when you save it.")
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(Color.accentColor.opacity(0.12))
            .overlay(Divider(), alignment: .bottom)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("offlineFileBadge")
        }
    }
}

// MARK: - Presenting the window

#if os(macOS)
/// Tools > Snapshots...: one window.
final class SnapshotsWindowController: NSObject, NSWindowDelegate {
    static let shared = SnapshotsWindowController()
    private var window: NSWindow?

    func show() {
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 560),
                             styleMask: [.titled, .closable, .resizable, .miniaturizable],
                             backing: .buffered, defer: false)
            w.title = "Snapshots"
            w.isReleasedWhenClosed = false
            w.delegate = self
            w.contentView = NSHostingView(rootView: ScrollView { SnapshotsView(snaps: .shared) }
                .frame(minWidth: 460, minHeight: 360)
                .preferredColorScheme(.dark))
            w.center()
            window = w
        }
        window?.makeKeyAndOrderFront(nil)
        CoreSnapshots.shared.reload()
    }
}
#else
/// The Snapshots sheet: half height, grows by dragging (owner: iOS sheets
/// at half height, Simple first).
enum SnapshotsSheet {
    static func show() {
        HalfSheet.present {
            NavigationStack {
                ScrollView { SnapshotsView(snaps: .shared) }
                    .navigationTitle("Snapshots")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) { Button("Done") { HalfSheet.dismiss() } }
                    }
            }
        }
    }
}

/// A SwiftUI sheet presented from UIKit at half height ([.medium, .large]).
enum HalfSheet {
    private static weak var shown: UIViewController?

    static func present<Content: View>(@ViewBuilder _ content: () -> Content) {
        let host = UIHostingController(rootView: content()
            .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
            .preferredColorScheme(.dark))
        host.modalPresentationStyle = .formSheet
        if let sheet = host.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        guard let presenter = UIApplication.shared.topViewController else { return }
        shown = host
        presenter.present(host, animated: true)
    }

    static func dismiss(then: @escaping () -> Void = {}) {
        guard let shown = shown, shown.presentingViewController != nil else { return then() }
        shown.dismiss(animated: true, completion: then)
    }
}
#endif

// MARK: - Snapshot files

/// Export and import of single snapshots (`.dspipreset` of kind snapshot).
enum SnapshotFiles {
    static var types: [UTType] {
        var types: [UTType] = []
        if let type = UTType(filenameExtension: PresetDocument.fileExtension) { types.append(type) }
        types.append(.json)
        return types
    }

    /// A name that is a file name: unsafe characters become "-".
    static func fileName(_ name: String) -> String {
        let bad = CharacterSet(charactersIn: "\\/:*?\"<>|")
        let safe = String(name.unicodeScalars.map { bad.contains($0) ? "-" : Character($0) })
        return (safe.isEmpty ? "Snapshot" : safe) + ".\(PresetDocument.fileExtension)"
    }

    static func export(_ s: SnapshotInfo, from snaps: CoreSnapshots) {
        snaps.exportText(s) { text in
            guard let text = text else { return }
            let name = fileName(s.name)
            #if os(macOS)
            let panel = NSSavePanel()
            panel.title = "Export Snapshot"
            panel.nameFieldStringValue = name
            panel.allowedContentTypes = types
            guard panel.runModal() == .OK, let url = panel.url else { return }
            do {
                try Data(text.utf8).write(to: url, options: .atomic)
                snaps.post("Snapshot exported to \(url.lastPathComponent).")
            } catch {
                snaps.post("Failed to write file: \(error.localizedDescription)", .error)
            }
            #else
            DocumentPicker.export(Data(text.utf8), fileName: name) { outcome in
                switch outcome {
                case .saved: snaps.post("Snapshot exported.")
                case .cancelled: break
                case .failed(let error): snaps.post("Failed to write file: \(error.localizedDescription)", .error)
                }
            }
            #endif
        }
    }

    static func importSnapshot(into snaps: CoreSnapshots) {
        #if os(macOS)
        let panel = NSOpenPanel()
        panel.title = "Import Snapshot"
        panel.allowedContentTypes = types
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        read(url, into: snaps)
        #else
        DocumentPicker.importFile(types: types) { url in read(url, into: snaps) }
        #endif
    }

    private static func read(_ url: URL, into snaps: CoreSnapshots) {
        do {
            snaps.importText(try OfflineFiles.readDocument(url))
        } catch {
            snaps.post("Failed to read file: \(error.localizedDescription)", .error)
        }
    }
}
