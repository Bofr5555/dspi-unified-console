//
//  DocumentPicker.swift
//  DSPi Console (iOS target)
//
//  WP SW17.  The Mac's File menu (Import/Export Filters, Import/Export Device
//  Configuration) on iOS: the system document picker stands in for
//  NSOpenPanel and NSSavePanel, and sheets stand in for the alerts with
//  checkboxes.  The files themselves are written and read by the code the Mac
//  uses (FileMenuActions in DSPi_ConsoleApp.swift, PresetDocumentFile), so a
//  file moves between the two apps unchanged.
//
//  Both directions copy: an import reads a copy the picker puts in the app's
//  sandbox, and an export is staged in the app's temporary directory and
//  copied out by the picker.  The app never holds a security-scoped URL.
//

#if os(iOS)
import SwiftUI
import UIKit
import UniformTypeIdentifiers

// MARK: - The document picker

enum DocumentPicker {
    enum ExportOutcome {
        case saved
        case cancelled
        case failed(Error)
    }

    enum PickerError: LocalizedError {
        case noWindow
        var errorDescription: String? { "There is no window to show the document picker in." }
    }

    /// The picker's delegate, held while the picker is on screen.
    private static var active: PickerDelegate?

    /// Lets the user pick one file of `types` and hands `read` a copy of it in
    /// the app's sandbox.  The copy is removed when `read` returns, so read it
    /// there.  Nothing happens on cancel.
    static func importFile(types: [UTType], read: @escaping (URL) -> Void) {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: true)
        picker.allowsMultipleSelection = false
        present(picker, onPick: { urls in
            guard let url = urls.first else { return }
            read(url)
            try? FileManager.default.removeItem(at: url)
        }, onCancel: {})
    }

    /// Lets the user save `data` as `fileName` wherever the picker offers
    /// (Files, iCloud Drive, another app's folder).
    static func export(_ data: Data, fileName: String, completion: @escaping (ExportOutcome) -> Void) {
        let url: URL
        do {
            url = try stage(data, fileName: fileName)
        } catch {
            completion(.failed(error))
            return
        }
        let cleanUp = { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }
        let picker = UIDocumentPickerViewController(forExporting: [url], asCopy: true)
        let shown = present(picker, onPick: { _ in
            cleanUp()
            completion(.saved)
        }, onCancel: {
            cleanUp()
            completion(.cancelled)
        })
        if !shown {
            cleanUp()
            completion(.failed(PickerError.noWindow))
        }
    }

    /// Writes `data` to a directory of its own in the app's temporary
    /// directory, under the name the picker will offer.
    static func stage(_ data: Data, fileName: String) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("Export-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(fileName)
        try data.write(to: url, options: .atomic)
        return url
    }

    @discardableResult
    private static func present(_ picker: UIDocumentPickerViewController,
                                onPick: @escaping ([URL]) -> Void,
                                onCancel: @escaping () -> Void) -> Bool {
        guard let presenter = UIApplication.shared.topViewController else { return false }
        let delegate = PickerDelegate(onPick: onPick, onCancel: onCancel) { active = nil }
        active = delegate
        picker.delegate = delegate
        presenter.present(picker, animated: true)
        return true
    }
}

private final class PickerDelegate: NSObject, UIDocumentPickerDelegate {
    private let onPick: ([URL]) -> Void
    private let onCancel: () -> Void
    private let release: () -> Void

    init(onPick: @escaping ([URL]) -> Void, onCancel: @escaping () -> Void, release: @escaping () -> Void) {
        self.onPick = onPick
        self.onCancel = onCancel
        self.release = release
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        afterDismissal(of: controller) { self.onPick(urls) }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        afterDismissal(of: controller) { self.onCancel() }
    }

    /// The picker dismisses itself; what follows (a sheet, an alert) is
    /// presented once it has gone, or UIKit refuses to present it.
    private func afterDismissal(of controller: UIViewController, _ body: @escaping () -> Void) {
        let finish = {
            body()
            self.release()
        }
        if let transition = controller.transitionCoordinator {
            transition.animate(alongsideTransition: nil) { _ in finish() }
        } else {
            DispatchQueue.main.async(execute: finish)
        }
    }
}

// MARK: - The import's sheets

/// The Mac's import alerts (the channel checkboxes, the configuration's
/// options) and its progress sheet, as UIKit-presented SwiftUI sheets.  On an
/// iPhone they open at half height and grow by dragging; on an iPad they are
/// form sheets.  Text sizes stop where the rest of the app stops
/// (IOSApp.swift), so the sheets do not grow past their layout.
enum FileImportSheets {
    /// Which channels to import to; `onImport` gets their unified (EQ)
    /// channel indices.  Nothing happens on cancel.
    static func chooseChannels(message: String, choices: [FileMenuActions.ImportChannelChoice],
                               onImport: @escaping ([Int]) -> Void) {
        present { box in
            ChannelChooser(message: message, choices: choices,
                           cancel: { box.close() },
                           commit: { chosen in box.close { onImport(chosen) } })
        }
    }

    /// What a configuration import brings in, as the Mac asks it: audio
    /// processing always, volume and hardware I/O by choice.
    static func askImportOptions(prompt: String, onImport: @escaping (PresetApplyOptions) -> Void) {
        present { box in
            ImportOptionsForm(prompt: prompt,
                              cancel: { box.close() },
                              commit: { options in box.close { onImport(options) } })
        }
    }

    /// "Writing settings to the device..." while a configuration applies.  It
    /// cannot be swiped away: the apply runs to the end either way.
    final class ProgressSheet {
        private let model = ProgressModel()
        private var box: SheetBox?

        func begin() {
            box = FileImportSheets.present(style: .overFullScreen) { [model] _ in ProgressCard(model: model) }
        }

        func update(_ fraction: Double) { model.fraction = fraction }

        func end(_ then: @escaping () -> Void) {
            guard let box else { return then() }
            box.close(then: then)
        }
    }

    @discardableResult
    private static func present<Content: View>(style: UIModalPresentationStyle = .formSheet,
                                               _ build: (SheetBox) -> Content) -> SheetBox {
        let box = SheetBox()
        let host = UIHostingController(rootView: build(box)
            .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
            .preferredColorScheme(.dark))
        host.modalPresentationStyle = style
        if style == .overFullScreen {
            host.modalTransitionStyle = .crossDissolve
            host.view.backgroundColor = .clear
            host.isModalInPresentation = true
        } else if let sheet = host.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        guard let presenter = UIApplication.shared.topViewController else {
            box.shown = true   // nothing to dismiss: close runs its follow-up at once
            return box
        }
        box.controller = host
        presenter.present(host, animated: true) { box.didShow() }
        return box
    }
}

/// Closes a presented sheet, then runs what follows.  A close asked for while
/// the sheet is still animating in waits for it, since UIKit drops a dismiss
/// issued during a presentation (and its completion with it).
private final class SheetBox {
    weak var controller: UIViewController?
    var shown = false
    private var pendingClose: (() -> Void)?

    func didShow() {
        shown = true
        if let then = pendingClose {
            pendingClose = nil
            close(then: then)
        }
    }

    func close(then: @escaping () -> Void = {}) {
        guard shown else {
            pendingClose = then
            return
        }
        guard let controller, controller.presentingViewController != nil else { return then() }
        controller.dismiss(animated: true, completion: then)
    }
}

private struct ChannelChooser: View {
    let message: String
    @State var choices: [FileMenuActions.ImportChannelChoice]
    let cancel: () -> Void
    let commit: ([Int]) -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach($choices) { $choice in
                        Toggle(choice.name, isOn: $choice.selected)
                    }
                } header: {
                    Text(message).textCase(nil)
                }
            }
            .navigationTitle("Import Filters")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: cancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") { commit(choices.filter(\.selected).map(\.id)) }
                }
            }
        }
    }
}

private struct ImportOptionsForm: View {
    let prompt: String
    let cancel: () -> Void
    let commit: (PresetApplyOptions) -> Void
    @State private var volumeLevels = false
    @State private var hardwareIO = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(prompt)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Section {
                    Toggle(isOn: $volumeLevels) {
                        labelled("Volume levels", "Master and listening volume")
                    }
                    Toggle(isOn: $hardwareIO) {
                        labelled("Hardware I/O", "GPIO pins, clocks, ADAT, inputs, output limiters")
                    }
                } header: {
                    Text("Also import")
                }
            }
            .navigationTitle("Import Device Configuration")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: cancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") {
                        commit(PresetApplyOptions(audioProcessing: true,
                                                  volumeLevels: volumeLevels,
                                                  hardwareIO: hardwareIO))
                    }
                }
            }
        }
    }

    private func labelled(_ title: String, _ detail: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
            Text(detail)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

private final class ProgressModel: ObservableObject {
    @Published var fraction = 0.0
}

private struct ProgressCard: View {
    @ObservedObject var model: ProgressModel

    var body: some View {
        ZStack {
            Color.black.opacity(0.35).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 12) {
                Text("Writing settings to the device...")
                ProgressView(value: model.fraction)
            }
            .padding(24)
            .frame(maxWidth: 320)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
            .padding(.horizontal, 24)
        }
    }
}
#endif
