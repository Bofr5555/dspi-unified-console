//
//  EventsView.swift
//  DSPi Console
//
//  -- WP P2SW -- The Events window (macOS: Tools > Events...; iOS: a sheet
//  from the Tools menu), as Qt's EventsWindow and Android's History screen
//  (P2QT, P2AN): this console's own event log of the open device, and, over
//  DSPi Link to a hub with the `events` family, the hub's shared history
//  "On the hub" with who did it (`by`), live through `event.added` while it
//  shows.  The calls are HubServerModel's; this only draws them.
//

import SwiftUI
import Combine
import dspi_core
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// One line of either log.
struct EventLine: Identifiable, Equatable {
    let id: String
    let timeMs: Int64
    let text: String
    /// Who did it ("" for this console's own log, or nobody).
    let by: String

    init(_ e: EventEntry) {
        id = "l\(e.id)"
        timeMs = e.timeMs
        text = e.text
        by = ""
    }

    init(_ r: HubEventRecord) {
        id = "h\(r.id)"
        timeMs = r.timeMs
        text = r.text
        by = r.by
    }

    var time: String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(timeMs) / 1000))
    }
}

/// What the window shows.  Main thread.
final class EventsModel: ObservableObject {
    static let shared = EventsModel(hub: .shared)

    enum Side: String, CaseIterable, Identifiable {
        case local, hub
        var id: String { rawValue }
        var title: String { self == .local ? "This console" : "On the hub" }
    }

    @Published var side: Side = .local { didSet { if side != oldValue { reload() } } }
    @Published private(set) var lines: [EventLine] = []
    @Published private(set) var error: String?

    let hub: HubServerModel
    private var cancellables = Set<AnyCancellable>()
    private var showing = false

    init(hub: HubServerModel) {
        self.hub = hub
        hub.eventAdded.receive(on: DispatchQueue.main)
            .sink { [weak self] r in self?.added(r) }.store(in: &cancellables)
        hub.$caps.map(\.events).removeDuplicates().receive(on: DispatchQueue.main)
            .sink { [weak self] on in
                guard let self = self else { return }
                if !on, self.side == .hub { self.side = .local }
                if self.showing { self.hub.watchEvents(on && self.side == .hub) }
            }.store(in: &cancellables)
    }

    /// The hub side is offered: a hub with the family.
    var hubOffered: Bool { hub.caps.events }

    func appeared() {
        showing = true
        reload()
    }

    func disappeared() {
        showing = false
        hub.watchEvents(false)
    }

    func reload() {
        error = nil
        switch side {
        case .local:
            hub.watchEvents(false)
            hub.localEvents { [weak self] list in
                guard self?.side == .local else { return }
                self?.lines = list.map(EventLine.init)
            }
        case .hub:
            hub.watchEvents(showing)
            hub.hubEvents { [weak self] result in
                guard let self = self, self.side == .hub else { return }
                switch result {
                case .success(let page): self.lines = page.records.reversed().map(EventLine.init)
                case .failure(let e):
                    self.lines = []
                    self.error = HubServerModel.words(e)
                }
            }
        }
    }

    /// A record the hub added while watched: newest first.
    func added(_ r: HubEventRecord) {
        guard side == .hub else { return }
        let line = EventLine(r)
        guard !lines.contains(where: { $0.id == line.id }) else { return }
        lines.insert(line, at: 0)
    }
}

struct EventsView: View {
    @ObservedObject var model: EventsModel
    @ObservedObject var hub: HubServerModel

    init(model: EventsModel = .shared) {
        self.model = model
        self.hub = model.hub
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                if model.hubOffered {
                    Picker("Events", selection: $model.side) {
                        ForEach(EventsModel.Side.allCases) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .frame(maxWidth: 300)
                    .accessibilityIdentifier("eventsSide")
                } else {
                    Text("This console").font(.headline)
                }
                Spacer()
                Button("Refresh") { model.reload() }
                    .accessibilityIdentifier("eventsRefresh")
            }
            if !hub.connected {
                Text("Connect a device to see its events.").foregroundColor(.secondary)
            } else if let error = model.error {
                Text(error).foregroundColor(.orange).accessibilityIdentifier("eventsError")
            } else if model.lines.isEmpty {
                Text("No events yet.").foregroundColor(.secondary).accessibilityIdentifier("eventsEmpty")
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(model.lines) { line in
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Text(line.time).font(.caption.monospacedDigit()).foregroundColor(.secondary)
                            Text(line.text).font(.callout)
                            Spacer(minLength: 0)
                            if !line.by.isEmpty {
                                Text(line.by).font(.caption).foregroundColor(.secondary)
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            }
            .accessibilityIdentifier("eventsList")
        }
        .padding(16)
        .frame(minWidth: 320, idealWidth: 560, maxWidth: .infinity, minHeight: 240, alignment: .topLeading)
        .onAppear { model.appeared() }
        .onDisappear { model.disappeared() }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("eventsView")
    }
}

#if os(macOS)
/// Tools > Events...: one window.
final class EventsWindowController: NSObject, NSWindowDelegate {
    static let shared = EventsWindowController()
    private var window: NSWindow?

    func show() {
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 480),
                             styleMask: [.titled, .closable, .resizable, .miniaturizable],
                             backing: .buffered, defer: false)
            w.title = "Events"
            w.isReleasedWhenClosed = false
            w.delegate = self
            w.contentView = NSHostingView(rootView: EventsView().preferredColorScheme(.dark))
            w.center()
            window = w
        }
        window?.makeKeyAndOrderFront(nil)
        EventsModel.shared.reload()
    }
}
#else
/// Tools > Events...: a sheet at half height.
enum EventsSheet {
    static func show() {
        HalfSheet.present {
            NavigationStack {
                EventsView()
                    .navigationTitle("Events")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) { Button("Done") { HalfSheet.dismiss() } }
                    }
            }
        }
    }
}
#endif
