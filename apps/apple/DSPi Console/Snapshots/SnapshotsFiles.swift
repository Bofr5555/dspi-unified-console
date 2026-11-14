//
//  SnapshotsFiles.swift
//  DSPi Console
//
//  WP P1SW1.  Offline files and templates (PRO1 W7, P1OF) and the import
//  preview (PRO1 W5, W6, W8; P1DF, P1SA), as Qt's FilesController and
//  Android's FileMenuActions/ImportPreviewDialog (P1QT1, P1AN1, P1DCFIX):
//
//  - Open File... opens a `.dspipreset` with no device: the app switches
//    (after its unsaved-changes prompt) to a device of its own seeded from
//    the document (`CoreBackedTransport` `.document`, the core's
//    `openDocument`), and the main window says "File: <name>".  Save File...
//    writes it back (`saveDocument`); Save as Template... files the open
//    device or document in the app's templates folder, where Open
//    Template... lists it beside the shipped ones (`templateList`).
//  - Import Device Configuration... through a core shows what the file
//    changes first (`configPreview`), a choice per block, and "Apply with
//    rollback" (fixed on over DSPi Link to a hub with the guard): a guarded
//    import (`applyGuardedImport`) and the countdown bar, or a plain one
//    with its confirmation token (`configImportConfirmed`, P1TK).  The result
//    is a notice line, not a modal.
//

import Foundation
import SwiftUI
import dspi_core
import dspi_constants
#if os(macOS)
import AppKit
import UniformTypeIdentifiers
#else
import UIKit
import UniformTypeIdentifiers
#endif

enum OfflineFiles {
    /// The core refuses a document over 4 MB; one byte more is enough for it
    /// to say so.
    static let readLimit = (4 << 20) + 1

    enum ReadError: LocalizedError {
        case notText
        var errorDescription: String? { "The file is not a text document." }
    }

    /// A document file's text, at most `readLimit` bytes of it.
    static func readDocument(_ url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: readLimit) ?? Data()
        guard let text = String(data: data, encoding: .utf8) else { throw ReadError.notText }
        return text
    }

    /// The user's templates: the app's own folder.
    static var templatesFolder: URL? { CoreSnapshots.appFolder("templates") }

    /// The input-pair links the export carries (the app's own setting).
    static func links(_ vm: DSPViewModel) -> UInt8 {
        (0..<DSPViewModel.inputPairCount).reduce(UInt8(0)) { mask, pair in
            vm.isInputPairLinked(pair) ? mask | UInt8(1 << pair) : mask
        }
    }

    // MARK: Open

    /// Open `text` offline as a device of its own, named `fileName`.  Main thread.
    static func open(text: String, fileName: String,
                     vm: DSPViewModel = AppState.shared.viewModel,
                     transport: CoreBackedTransport = AppState.shared.documentTransport,
                     snaps: CoreSnapshots = .shared) {
        transport.documentOpened = { [weak snaps] result in
            guard let snaps = snaps else { return }
            switch result {
            case .success(let json):
                let o = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
                let name = (o["name"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? fileName
                let notes = o["notes"] as? [String] ?? []
                let text = "Opened \u{201C}\(name)\u{201D} with no device."
                snaps.post(notes.isEmpty ? text : text + " " + notes.joined(separator: " "),
                           notes.isEmpty ? .info : .warning)
            case .failure(let e):
                snaps.post("The file could not be opened: \(CoreSnapshots.words(e))", .error)
            }
        }
        let device = transport.prepareDocument(text: text, fileName: fileName)
        vm.switchToDevice(device)
    }

    /// File > Open File...
    static func openFile() {
        #if os(macOS)
        let panel = NSOpenPanel()
        panel.title = "Open File"
        panel.allowedContentTypes = SnapshotFiles.types
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        openURL(url)
        #else
        DocumentPicker.importFile(types: SnapshotFiles.types) { url in openURL(url) }
        #endif
    }

    private static func openURL(_ url: URL) {
        do {
            open(text: try readDocument(url), fileName: url.lastPathComponent)
        } catch {
            CoreSnapshots.shared.post("Failed to read file: \(error.localizedDescription)", .error)
        }
    }

    // MARK: Templates

    /// The shipped templates, then the user's (`templateList`; reads files:
    /// call off main).
    static func templates() -> [TemplateEntry] {
        TemplateEntry.parse(templateList(userDir: templatesFolder?.path ?? ""))
    }

    /// File > Open Template...: the list, then the chosen one offline.
    static func showTemplates() {
        CoreSnapshots.work.async {
            let list = templates()
            DispatchQueue.main.async {
                TemplatePicker.show(list) { entry in openTemplate(entry) }
            }
        }
    }

    static func openTemplate(_ t: TemplateEntry) {
        let dir = templatesFolder?.path ?? ""
        CoreSnapshots.work.async {
            let result = Result { try templateText(id: t.id, userDir: dir) }
            DispatchQueue.main.async {
                switch result {
                case .success(let text): open(text: text, fileName: TemplateEntry.fileName(t.name))
                case .failure(let e):
                    let words = (e as? DspiError).map(CoreSnapshots.words) ?? e.localizedDescription
                    CoreSnapshots.shared.post("The template could not be read: \(words)", .error)
                }
            }
        }
    }

    // MARK: Save

    /// File > Save File...: the open document as it now stands.
    static func saveFile(vm: DSPViewModel = AppState.shared.viewModel, snaps: CoreSnapshots = .shared) {
        guard let source = vm.transport.coreStateSource, let doc = snaps.offline else {
            return snaps.post("No file is open. Open one with Open File...", .warning)
        }
        let linkMask = links(vm)
        let version = GeneralSettingsTab.appVersion
        CoreSnapshots.work.async {
            let result = source.call { try $0.saveDocument(appVersion: version, links: linkMask) }
            DispatchQueue.main.async {
                switch result {
                case .success(let text): write(text, suggested: doc.fileName.isEmpty ? doc.name : doc.fileName,
                                               title: "Save File", snaps: snaps)
                case .failure(let e): snaps.post(CoreSnapshots.words(e), .error)
                }
            }
        }
    }

    private static func write(_ text: String, suggested: String, title: String, snaps: CoreSnapshots) {
        var name = suggested
        if !name.lowercased().hasSuffix(".\(PresetDocument.fileExtension)") {
            name += ".\(PresetDocument.fileExtension)"
        }
        #if os(macOS)
        let panel = NSSavePanel()
        panel.title = title
        panel.nameFieldStringValue = name
        panel.allowedContentTypes = SnapshotFiles.types
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try Data(text.utf8).write(to: url, options: .atomic)
            snaps.post("Saved \(url.lastPathComponent).")
        } catch {
            snaps.post("Failed to write file: \(error.localizedDescription)", .error)
        }
        #else
        DocumentPicker.export(Data(text.utf8), fileName: name) { outcome in
            switch outcome {
            case .saved: snaps.post("Saved \(name).")
            case .cancelled: break
            case .failed(let error): snaps.post("Failed to write file: \(error.localizedDescription)", .error)
            }
        }
        #endif
    }

    /// File > Save as Template...: the name asked for, then the template filed.
    static func askSaveTemplate(vm: DSPViewModel = AppState.shared.viewModel, snaps: CoreSnapshots = .shared) {
        guard vm.isDeviceConnected, vm.transport.coreStateSource != nil else {
            return snaps.post("No device or file is open.", .warning)
        }
        let suggested = snaps.offline?.name ?? ""
        #if os(macOS)
        let alert = NSAlert()
        alert.messageText = "Save as Template"
        alert.informativeText = "The settings as they are now, kept on this console as a template. Open it later with Open Template..."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = suggested
        field.placeholderString = "Name"
        alert.accessoryView = field
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        saveTemplate(name: field.stringValue, vm: vm, snaps: snaps)
        #else
        let alert = UIAlertController(title: "Save as Template",
                                      message: "The settings as they are now, kept on this device as a template.",
                                      preferredStyle: .alert)
        alert.addTextField { $0.text = suggested; $0.placeholder = "Name" }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Save", style: .default) { [weak alert] _ in
            saveTemplate(name: alert?.textFields?.first?.text ?? "", vm: vm, snaps: snaps)
        })
        UIApplication.shared.topViewController?.present(alert, animated: true)
        #endif
    }

    static func saveTemplate(name raw: String, vm: DSPViewModel = AppState.shared.viewModel,
                             snaps: CoreSnapshots = .shared, folder: URL? = OfflineFiles.templatesFolder,
                             done: @escaping (Bool) -> Void = { _ in }) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = String((trimmed.isEmpty ? "Template" : trimmed).prefix(CoreSnapshots.nameMax))
        guard let source = vm.transport.coreStateSource, let folder = folder else {
            snaps.post("Templates need a device or a file on the core.", .warning)
            return done(false)
        }
        let linkMask = links(vm)
        let version = GeneralSettingsTab.appVersion
        CoreSnapshots.work.async {
            let made = source.call { try $0.saveAsTemplate(name: name, appVersion: version, links: linkMask) }
            var failure: String?
            switch made {
            case .success(let text):
                do {
                    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                    try Data(text.utf8).write(to: folder.appendingPathComponent(TemplateEntry.fileName(name)),
                                              options: .atomic)
                } catch {
                    failure = "The template was not saved: \(error.localizedDescription)"
                }
            case .failure(let e):
                failure = "The template was not saved: \(CoreSnapshots.words(e))"
            }
            DispatchQueue.main.async {
                if let failure = failure {
                    snaps.post(failure, .error)
                } else {
                    snaps.post("Template \u{201C}\(name)\u{201D} saved. Open it from Open Template...")
                }
                done(failure == nil)
            }
        }
    }
}

// MARK: - The template list

struct TemplatePickerView: View {
    let entries: [TemplateEntry]
    let open: (TemplateEntry) -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Open Template").font(.title3.weight(.semibold))
            Text("A template opens with no device, as a file. Save it as a file, or import it into a device.")
                .font(.callout)
                .foregroundColor(.secondary)
            if entries.isEmpty {
                Text("No templates yet. Save one with Save as Template...").foregroundColor(.secondary)
            }
            ForEach(entries) { t in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t.name).fontWeight(.semibold)
                        Text([t.builtin ? "Shipped" : "Yours", t.platform].filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.caption)
                            .foregroundColor(.secondary)
                        if !t.status.isEmpty { Text(t.status).font(.caption).foregroundColor(.orange) }
                    }
                    Spacer()
                    Button("Open") { open(t) }
                }
            }
            HStack {
                Spacer()
                Button("Cancel", action: cancel)
            }
        }
        .padding(16)
        .frame(minWidth: 360, alignment: .topLeading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("templatePicker")
    }
}

enum TemplatePicker {
    #if os(macOS)
    private static var window: NSWindow?
    #endif

    static func show(_ entries: [TemplateEntry], open: @escaping (TemplateEntry) -> Void) {
        #if os(macOS)
        let w = window ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 360),
                                   styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        w.title = "Open Template"
        w.isReleasedWhenClosed = false
        w.contentView = NSHostingView(rootView: ScrollView {
            TemplatePickerView(entries: entries,
                               open: { t in w.orderOut(nil); open(t) },
                               cancel: { w.orderOut(nil) })
        }.preferredColorScheme(.dark))
        w.center()
        window = w
        w.makeKeyAndOrderFront(nil)
        #else
        HalfSheet.present {
            ScrollView {
                TemplatePickerView(entries: entries,
                                   open: { t in HalfSheet.dismiss { open(t) } },
                                   cancel: { HalfSheet.dismiss() })
            }
        }
        #endif
    }
}

// MARK: - The import preview

/// What the preview shows and the user chose.  Main thread.
final class ImportPreviewModel: ObservableObject, Identifiable {
    let text: String
    let name: String
    let intro: String
    let diff: SnapshotDiff
    let guardDefault: Bool
    @Published var selected: Set<ImportBlock>
    @Published var guarded = false

    init(text: String, name: String, intro: String, diff: SnapshotDiff, guardDefault: Bool) {
        self.text = text
        self.name = name
        self.intro = intro
        self.diff = diff
        self.guardDefault = guardDefault
        selected = ImportBlock.defaults(diff)
    }

    /// The rows of the chosen blocks.
    var chosenRows: [SnapshotDiffRow] {
        let keys = Set(selected.map(\.key))
        return diff.rows.filter { keys.contains($0.block) }
    }

    /// With a way back: the user's choice, or always over DSPi Link.
    var withRollback: Bool { guarded || guardDefault }

    var countLine: String {
        if !diff.ok { return diff.error }
        if diff.rows.isEmpty { return "Nothing in this file differs from the device." }
        return "\(CoreSnapshots.changesLine(diff.rows.count)) from the device; \(CoreSnapshots.changesLine(chosenRows.count)) chosen."
    }
}

struct ImportPreviewView: View {
    @ObservedObject var model: ImportPreviewModel
    let cancel: () -> Void
    let apply: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Import").font(.title3.weight(.semibold))
                Spacer()
                Button("Cancel", action: cancel)
                Button(model.withRollback ? "Apply with rollback" : "Import", action: apply)
                    .buttonStyle(.borderedProminent)
                    .disabled(model.selected.isEmpty)
                    .accessibilityIdentifier("importPreviewApply")
            }
            if !model.name.isEmpty { Text(model.name).font(.headline) }
            if !model.intro.isEmpty { Text(model.intro).font(.callout).foregroundColor(.secondary) }
            Text(model.countLine).font(.callout).foregroundColor(.secondary)
            Text("Bring in").font(.subheadline.weight(.semibold))
            let counts = model.diff.countByBlock
            ForEach(ImportBlock.allCases) { b in
                Toggle(isOn: Binding(get: { model.selected.contains(b) },
                                     set: { on in if on { model.selected.insert(b) } else { model.selected.remove(b) } })) {
                    HStack {
                        Text(b.title)
                        Spacer()
                        if model.diff.ok {
                            let n = counts[b.key] ?? 0
                            Text(n == 0 ? "no changes" : CoreSnapshots.changesLine(n))
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                }
                .toggleStyle(.checkbox)
            }
            Text("Way back").font(.subheadline.weight(.semibold))
            Toggle(isOn: Binding(get: { model.withRollback }, set: { model.guarded = $0 })) {
                Text(model.guardDefault ? "Apply with rollback (always over DSPi Link)" : "Apply with rollback")
            }
            .toggleStyle(.checkbox)
            .disabled(model.guardDefault)
            .accessibilityIdentifier("importPreviewRollback")
            Text(CoreSnapshots.rollbackExplained).font(.caption).foregroundColor(.secondary)
            if !model.chosenRows.isEmpty {
                Text("What changes").font(.subheadline.weight(.semibold))
                DiffRowsList(rows: model.chosenRows)
            }
            if !model.diff.notes.isEmpty {
                Text("Notes").font(.subheadline.weight(.semibold))
                ForEach(Array(model.diff.notes.enumerated()), id: \.offset) { _, n in
                    Text(n).font(.caption)
                }
            }
        }
        .padding(16)
        .frame(minWidth: 380, alignment: .topLeading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("importPreview")
    }
}

enum ImportPreview {
    #if os(macOS)
    private static var window: NSWindow?
    #endif

    /// Through a core: the preview of `text` against the device, then the
    /// user's choice applied.  Main thread.
    static func begin(text: String, doc: PresetDocument, vm: DSPViewModel,
                      snaps: CoreSnapshots = .shared) {
        guard let source = vm.transport.coreStateSource else { return }
        let intro = FileMenuActions.configurationImportPrompt(doc, vm: vm)
            .components(separatedBy: "\n\nEQ, crossover").first ?? ""
        let name = doc.meta.name ?? ""
        CoreSnapshots.work.async {
            let preview = source.call {
                ($0.configPreview(text: text, options: ImportBlock.options(Set(ImportBlock.allCases)),
                                  eqChannels: allEqChannels), $0.applyGuardDefault())
            }
            DispatchQueue.main.async {
                switch preview {
                case .success(let (json, guardDefault)):
                    let model = ImportPreviewModel(text: text, name: name, intro: intro,
                                                   diff: SnapshotDiff.parse(json), guardDefault: guardDefault)
                    show(model, vm: vm, snaps: snaps)
                case .failure(let e):
                    snaps.post("The file could not be previewed: \(CoreSnapshots.words(e))", .error)
                }
            }
        }
    }

    private static func show(_ model: ImportPreviewModel, vm: DSPViewModel, snaps: CoreSnapshots) {
        #if os(macOS)
        let w = window ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 600),
                                   styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        w.title = "Import Device Configuration"
        w.isReleasedWhenClosed = false
        w.contentView = NSHostingView(rootView: ScrollView {
            ImportPreviewView(model: model,
                              cancel: { w.orderOut(nil) },
                              apply: { w.orderOut(nil); apply(model, vm: vm, snaps: snaps) })
        }.preferredColorScheme(.dark))
        w.center()
        window = w
        w.makeKeyAndOrderFront(nil)
        #else
        HalfSheet.present {
            ScrollView {
                ImportPreviewView(model: model,
                                  cancel: { HalfSheet.dismiss() },
                                  apply: { HalfSheet.dismiss { apply(model, vm: vm, snaps: snaps) } })
            }
        }
        #endif
    }

    /// The choice applied: guarded (the countdown bar), or plain with its
    /// confirmation token.
    static func apply(_ model: ImportPreviewModel, vm: DSPViewModel, snaps: CoreSnapshots = .shared,
                      done: @escaping (Bool) -> Void = { _ in }) {
        let options = ImportBlock.options(model.selected)
        if model.withRollback {
            snaps.beginGuardedImport(model.text, options: options, done: done)
            return
        }
        guard let source = vm.transport.coreStateSource else { return done(false) }
        let text = model.text
        CoreSnapshots.work.async {
            // As a write: behind the queued writes, then the state and the
            // journal published (the import ended the undo history).
            let reply = source.writeAndWait { core -> String in
                let token = core.confirmToken(op: .configImport)
                return core.configImportConfirmed(text: text, options: options, eqChannels: allEqChannels,
                                                  token: token)
            }
            let report: PresetApplyReport
            var links: UInt8?
            switch reply {
            case .success(let json):
                let outcome = PresetDocumentApply.report(fromCoreReply: json)
                report = outcome.0
                links = outcome.links
            case .failure(let e):
                report = PresetApplyReport()
                if case .Locked = e { report.lockedOut = true } else { report.skip(CoreSnapshots.words(e)) }
                report.coreClean = false
            }
            if report.coreLines != nil { vm.applyCoreReread(source) }
            DispatchQueue.main.async {
                if let links = links {
                    for pair in 0..<DSPViewModel.inputPairCount {
                        let linked = links & (1 << UInt8(pair)) != 0
                        if vm.isInputPairLinked(pair) != linked { vm.setInputPairLinked(pair, linked) }
                    }
                }
                if report.lockedOut {
                    snaps.post("The configuration was not applied: \(DeviceLock.lockedMessage).", .error)
                } else if let lines = report.coreLines {
                    snaps.post(lines.joined(separator: " "), report.isClean ? .info : .warning)
                } else {
                    snaps.post(report.skipped.joined(separator: " "), .error)
                }
                done(report.coreLines != nil)
            }
        }
    }
}
