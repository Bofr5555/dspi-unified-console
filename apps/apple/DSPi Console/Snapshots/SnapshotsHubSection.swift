//
//  SnapshotsHubSection.swift
//  DSPi Console
//
//  -- WP P2SW -- The Snapshots window's "On the hub" section (PLAN-P2: the
//  hub snapshot store), as Qt's and Android's (P2QT, P2AN).  Over DSPi Link to
//  a hub with `hub_snapshots`, the core merges both sides' lists
//  (`snapshotListMerged`): the hub wins on a differing id, the local one kept
//  as a local copy, never overwritten.  Here: the hub's rows with Recall (this
//  console's copy, or the hub's copied here first, then the usual recall with
//  its rollback), Copy here, Rename and Delete from the hub; and Save to the
//  hub for a snapshot only this console has.  `snapshot.changed` reloads.
//  Nothing shows on USB or on a hub without the family.
//

import SwiftUI
import Combine
import dspi_core

/// The merged list and its actions.  Main thread.
final class SnapshotsHubModel: ObservableObject {
    /// The app's: the Snapshots window's.
    static let shared = SnapshotsHubModel(hub: .shared, snaps: .shared)

    @Published private(set) var entries: [MergedSnapshot] = []
    /// The hub listed its side (false: it could not be asked).
    @Published private(set) var hubListed = false
    @Published private(set) var busy = false

    let hub: HubServerModel
    let snaps: CoreSnapshots
    private var cancellables = Set<AnyCancellable>()

    init(hub: HubServerModel, snaps: CoreSnapshots) {
        self.hub = hub
        self.snaps = snaps
        hub.$snapshotsGeneration.dropFirst().receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.reload() }.store(in: &cancellables)
        hub.$caps.map(\.hubSnapshots).removeDuplicates().receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.reload() }.store(in: &cancellables)
        snaps.$snapshots.dropFirst().receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.reload() }.store(in: &cancellables)
    }

    /// The section shows: a hub with the family.
    var shown: Bool { hub.caps.hubSnapshots }

    /// The hub's rows (on the hub, or on both sides).
    var onHub: [MergedSnapshot] { Self.onHub(entries) }
    /// The rows only this console has.
    var localOnly: [MergedSnapshot] { Self.localOnly(entries) }

    static func onHub(_ list: [MergedSnapshot]) -> [MergedSnapshot] {
        list.filter { $0.location != HubServerModel.whereLocal && !$0.localCopy }
    }

    static func localOnly(_ list: [MergedSnapshot]) -> [MergedSnapshot] {
        list.filter { $0.location == HubServerModel.whereLocal && !$0.localCopy }
    }

    /// "Also on this console" / "On the hub only", for a hub row.
    static func whereLine(_ e: MergedSnapshot) -> String {
        e.location == HubServerModel.whereBoth ? "Also on this console" : "On the hub only"
    }

    func reload() {
        guard shown else {
            if !entries.isEmpty { entries = [] }
            hubListed = false
            return
        }
        hub.merged { [weak self] list in
            self?.entries = list.entries.sorted { $0.snapshot.savedMs > $1.snapshot.savedMs }
            self?.hubListed = list.hubListed
        }
    }

    private func finished<T>(_ result: Result<T, DspiError>, ok: String) {
        busy = false
        switch result {
        case .success: snaps.post(ok)
        case .failure(let e): snaps.post(HubServerModel.words(e), .error)
        }
        snaps.reload()
        reload()
    }

    /// "Save to the hub": file this console's snapshot on the hub.
    func saveToHub(_ e: MergedSnapshot) {
        busy = true
        hub.saveToHub(e.snapshot.id) { [weak self] r in
            self?.finished(r, ok: "Saved \u{201C}\(e.snapshot.name)\u{201D} to the hub.")
        }
    }

    /// "Copy here": file the hub's snapshot in this console's own store.
    func copyHere(_ e: MergedSnapshot) {
        busy = true
        hub.copyHere(e.snapshot.id) { [weak self] r in
            self?.finished(r, ok: "Copied \u{201C}\(e.snapshot.name)\u{201D} to this console.")
        }
    }

    func rename(_ e: MergedSnapshot, to name: String) {
        busy = true
        hub.renameOnHub(e.snapshot.id, name) { [weak self] r in self?.finished(r, ok: "Renamed on the hub.") }
    }

    /// Remove it from the hub (Admin); this console's own copy stays.
    func delete(_ e: MergedSnapshot) {
        busy = true
        hub.deleteFromHub(e.snapshot.id) { [weak self] r in
            self?.finished(r, ok: "Removed \u{201C}\(e.snapshot.name)\u{201D} from the hub.")
        }
    }

    /// Recall: this console's copy, or the hub's copied here first, then the
    /// usual recall (with its rollback over DSPi Link).
    func recall(_ e: MergedSnapshot) {
        if e.location == HubServerModel.whereBoth,
           let local = snaps.snapshots.first(where: { $0.id == e.snapshot.id }) {
            snaps.recall(local)
            return
        }
        busy = true
        hub.copyHere(e.snapshot.id) { [weak self] r in
            guard let self = self else { return }
            self.busy = false
            switch r {
            case .success(let copy):
                self.snaps.reload()
                self.snaps.recall(copy)
            case .failure(let err):
                self.snaps.post(HubServerModel.words(err), .error)
            }
            self.reload()
        }
    }
}

/// The section under the Snapshots list.
struct SnapshotsHubSection: View {
    @ObservedObject var model: SnapshotsHubModel
    @ObservedObject var hub: HubServerModel
    let advanced: Bool
    @State private var renaming: MergedSnapshot?
    @State private var renameText = ""
    @State private var deleting: MergedSnapshot?

    init(model: SnapshotsHubModel, advanced: Bool) {
        self.model = model
        self.hub = model.hub
        self.advanced = advanced
    }

    var body: some View {
        if model.shown {
            GroupBox {
                VStack(alignment: .leading, spacing: 6) {
                    if model.onHub.isEmpty {
                        Text(model.hubListed ? "No snapshots of this device on the hub yet."
                                             : "The hub's snapshots could not be read.")
                            .foregroundColor(.secondary)
                            .accessibilityIdentifier("hubSnapshotsEmpty")
                    }
                    ForEach(model.onHub, id: \.snapshot.id) { e in hubRow(e) }
                    if advanced && !model.localOnly.isEmpty {
                        Divider()
                        Text("Only on this console").font(.caption).foregroundColor(.secondary)
                        ForEach(model.localOnly, id: \.snapshot.id) { e in
                            HStack {
                                Text(e.snapshot.name)
                                Spacer()
                                Button("Save to the Hub") { model.saveToHub(e) }
                                    .disabled(model.busy)
                                    .accessibilityIdentifier("snapshotSaveToHub")
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            } label: {
                Text("On the hub")
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("hubSnapshots")
            .onAppear { model.reload() }
            .alert("Rename on the Hub", isPresented: Binding(get: { renaming != nil },
                                                            set: { if !$0 { renaming = nil } })) {
                TextField("Name", text: $renameText)
                Button("Rename") {
                    if let e = renaming { model.rename(e, to: String(renameText.prefix(CoreSnapshots.nameMax))) }
                    renaming = nil
                }
                Button("Cancel", role: .cancel) { renaming = nil }
            }
            .confirmationDialog(deleting.map { "Remove \u{201C}\($0.snapshot.name)\u{201D} from the hub?" } ?? "",
                                isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                                titleVisibility: .visible) {
                Button("Remove from the Hub", role: .destructive) {
                    if let e = deleting { model.delete(e) }
                    deleting = nil
                }
                Button("Cancel", role: .cancel) { deleting = nil }
            } message: {
                Text("Every console that shares this hub loses it. A copy on this console stays.")
            }
        }
    }

    private func hubRow(_ e: MergedSnapshot) -> some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                Text(e.snapshot.name).fontWeight(.semibold)
                Text(SnapshotsHubModel.whereLine(e)).font(.caption).foregroundColor(.secondary)
            }
            Spacer()
            Button("Recall") { model.recall(e) }
                .disabled(model.busy || model.snaps.busy)
                .accessibilityIdentifier("hubSnapshotRecall")
            if advanced {
                Menu {
                    if e.location == HubServerModel.whereHub {
                        Button("Copy Here") { model.copyHere(e) }
                    }
                    Button("Rename on the Hub...") { renameText = e.snapshot.name; renaming = e }
                    Divider()
                    Button("Remove from the Hub...", role: .destructive) { deleting = e }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuIndicator(.hidden)
                .fixedSize()
                .disabled(model.busy)
                .accessibilityLabel("More")
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
    }
}
