//
//  IOSAutoEQMenu.swift
//  DSPi Console (iOS target)
//
//  WP SW18.  The Mac's AutoEQ menu (Browse Profiles, Favorite Profiles,
//  Clear Favorites, Update Database) as a submenu of the iOS Tools menu.
//  Update Database goes through the core (CG6, AutoEQ/AutoEQCoreUpdate.swift):
//  Rebuild from GitHub, Import File (the document picker) and Reset to
//  Built-in, with the Mac's alerts as UIKit alerts.
//

#if os(iOS)
import SwiftUI
import UniformTypeIdentifiers
import dspi_core

/// Tools > AutoEQ.
struct IOSAutoEQMenu: View {
    @ObservedObject var manager = AutoEQManager.shared
    let browse: () -> Void

    var body: some View {
        Menu {
            Button("Browse Profiles...", action: browse)
            Menu("Favorite Profiles") {
                ForEach(manager.favoriteProfiles) { entry in
                    Button("\(entry.manufacturer) \(entry.model)") {
                        manager.applyProfile(entry)
                    }
                }
                if manager.favoriteProfiles.isEmpty {
                    Button("No favorites yet") {}
                        .disabled(true)
                }
                Divider()
                Button("Clear Favorites", role: .destructive) {
                    manager.clearFavorites()
                }
                .disabled(manager.favoriteProfiles.isEmpty)
            }
            Divider()
            Button("Update Database...") { IOSAutoEQActions.updateDatabase() }
                .disabled(manager.isRebuilding)
        } label: {
            Label("AutoEQ", systemImage: "headphones")
        }
    }
}

/// The Mac's `AutoEQMenuActions`, over the core.
enum IOSAutoEQActions {
    static func updateDatabase(manager: AutoEQManager = .shared) {
        let alert = PlatformAlert()
        alert.messageText = "Update AutoEQ Database"
        alert.informativeText = AutoEQCoreUpdate.summary(manager) + "\n\nChoose an update method:"
        alert.addButton(withTitle: "Rebuild from GitHub")
        alert.addButton(withTitle: "Import File...")
        let canReset = manager.hasUserDatabase
        if canReset { alert.addButton(withTitle: "Reset to Built-in") }
        alert.addButton(withTitle: "Cancel")
        alert.present { index in
            switch index {
            case 0: confirmRebuild(manager: manager)
            case 1: importFile(manager: manager)
            case 2 where canReset: reset(manager: manager)
            default: break
            }
        }
    }

    /// Nothing reaches the network until the person has read the core's
    /// warning (what is fetched, how long, GitHub's limit) and said yes.
    private static func confirmRebuild(manager: AutoEQManager) {
        let alert = PlatformAlert()
        alert.messageText = "Rebuild AutoEQ Database"
        alert.informativeText = AutoEQCoreUpdate.rebuildWarning(manager) + "\n\nDo you wish to proceed?"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Rebuild")
        alert.addButton(withTitle: "Cancel")
        alert.present { index in
            if index == 0 { AutoEQRebuildSheet.shared.start(manager: manager) }
        }
    }

    private static func importFile(manager: AutoEQManager) {
        DocumentPicker.importFile(types: [.json]) { url in
            // The picker's copy goes when this returns: read it here.
            let data: Data
            do {
                data = try Data(contentsOf: url)
            } catch {
                showError("Failed to import database: \(error.localizedDescription)")
                return
            }
            AutoEQCoreUpdate.importDatabase(data, manager: manager) { result in
                switch result {
                case .success(let info):
                    showSuccess("Database updated successfully.\nEntries: \(info.entryCount)")
                case .failure(let error):
                    showError("Failed to import database: \(AutoEQCoreUpdate.message(error))")
                }
            }
        }
    }

    private static func reset(manager: AutoEQManager) {
        AutoEQCoreUpdate.resetToBuiltIn(manager) { result in
            switch result {
            case .success:
                showSuccess("Reset to built-in database.\nEntries: \(manager.entries.count)")
            case .failure(let error):
                showError("Failed to reset: \(AutoEQCoreUpdate.message(error))")
            }
        }
    }

    static func showSuccess(_ message: String) {
        let alert = PlatformAlert()
        alert.messageText = "Success"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.present()
    }

    static func showError(_ message: String) {
        let alert = PlatformAlert()
        alert.messageText = "Error"
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: "OK")
        alert.present()
    }
}

// MARK: - Rebuild progress

/// The rebuild's progress as a tool sheet.  Closing the sheet cancels the
/// rebuild (the database stays as it was); when it is over the sheet goes
/// and an alert says what came of it.
final class AutoEQRebuildSheet: IOSToolWindowController {
    static let shared = AutoEQRebuildSheet()
    private var rebuild: AutoEQCoreRebuild?

    func start(manager: AutoEQManager, fetch: AutoEqFetch = URLSessionAutoEqFetch()) {
        guard rebuild == nil else { return }
        let rebuild = AutoEQCoreRebuild(manager: manager)
        self.rebuild = rebuild
        present(title: "Rebuilding AutoEQ Database") {
            AutoEQRebuildProgressSheet(rebuild: rebuild)
        }
        rebuild.start(fetch: fetch) { [weak self] progress in
            self?.rebuild = nil
            self?.hide()
            // After the sheet has gone, or UIKit will not present the alert.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                if progress.succeeded {
                    IOSAutoEQActions.showSuccess(AutoEQCoreRebuild.summary(progress))
                } else {
                    IOSAutoEQActions.showError(AutoEQCoreRebuild.summary(progress))
                }
            }
        }
    }

    override func didHide() {
        // Done on the sheet while it runs: stop it; the poll then finishes it.
        rebuild?.cancel()
    }
}

struct AutoEQRebuildProgressSheet: View {
    @ObservedObject var rebuild: AutoEQCoreRebuild

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ProgressView(value: Double(rebuild.progress.fraction))
            Text(rebuild.progress.status)
                .font(.callout)
                .foregroundColor(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button("Cancel", role: .cancel) { rebuild.cancel() }
                .disabled(rebuild.progress.finished)
        }
        .padding(24)
        .frame(maxWidth: 480)
    }
}
#endif
