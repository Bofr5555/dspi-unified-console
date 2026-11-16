//
//  LinkBannerView.swift
//  DSPi Console
//
//  WP SW7.  The owner's rule for a lost DSPi Link connection, on Mac and iOS:
//  say so at once, over the main window, with the way out.  The text and the
//  button are the core's (CG10's `reconnecting` entry and its
//  `reconnecting.disconnect` action, `link_phase_text`); the hub's name and
//  the core's `close_reason` are the second line, as on Android.  The button
//  leaves the hub (`link_close`, through CoreLink.disconnect), also in the
//  middle of a dial.
//
//  Nothing here waits or retries: the core reconnects by itself, and while it
//  does the view model keeps the last values read (the transport holds the
//  device "connected", SW6) and the core refuses a write at once, which is
//  never replayed.  Qt's `link/LinkBanner.qml`, Android's
//  `ui/link/LinkBanner.kt`, the web's `LostLinkBanner`.
//
//  `LinkBannerCenter` is fed by every `CoreLink` (one per hub) on each look at
//  the core's status; a session that is not reconnecting has no entry.  Main
//  thread only.
//

import SwiftUI
import Combine
import dspi_core

/// One hub whose session the core is reconnecting.
struct LinkBannerEntry: Identifiable, Equatable {
    /// The session (its CoreLink).
    let id: ObjectIdentifier
    let hubName: String
    /// The core's wording for the phase (`link_phase_text`).
    let words: LinkPhaseText
    /// The core's `close_reason` of the lost connection ("" if none).
    let reason: String

    /// The first line: the core's text.
    var text: String { words.text }
    /// The way out's label: the core's.  The table always has one for
    /// `reconnecting` (a CG10 test holds it); the fallback is Qt's.
    var action: String { words.action ?? "Disconnect" }
    /// The second line: the hub, and why the connection went.
    var detail: String { reason.isEmpty ? hubName : "\(hubName): \(reason)" }
}

/// The lost sessions the banner shows.  Main thread.
final class LinkBannerCenter: ObservableObject {
    static let shared = LinkBannerCenter()

    @Published private(set) var entries: [LinkBannerEntry] = []
    /// Each entry's way out (`link_close`).
    private var ways: [ObjectIdentifier: () -> Void] = [:]

    /// A session's phase as the last look saw it: an entry while it is
    /// reconnecting, none otherwise.
    func report(_ id: ObjectIdentifier, hubName: String, phase: LinkPhase, pinSource: String,
                closeReason: String, disconnect: @escaping () -> Void) {
        guard phase == .reconnecting else { return clear(id) }
        let entry = LinkBannerEntry(id: id, hubName: hubName,
                                    words: linkPhaseText(phase: phase, pinSource: pinSource),
                                    reason: closeReason)
        ways[id] = disconnect
        if let i = entries.firstIndex(where: { $0.id == id }) {
            if entries[i] != entry { entries[i] = entry }
        } else {
            entries.append(entry)
        }
    }

    /// The session ended or is no longer reconnecting.
    func clear(_ id: ObjectIdentifier) {
        ways[id] = nil
        if entries.contains(where: { $0.id == id }) { entries.removeAll { $0.id == id } }
    }

    /// The banner's button: leave that hub.  The entry goes at once.
    func disconnect(_ id: ObjectIdentifier) {
        let way = ways[id]
        clear(id)
        way?()
    }
}

// -- WP SW7 -- What CoreLink tells the banner (Core/CoreLink.swift calls these).
extension CoreLink {
    /// Each look at the core's status.
    func reportToBanner(_ status: dspi_core.LinkStatus) {
        banner.report(ObjectIdentifier(self), hubName: hubName, phase: status.phase,
                      pinSource: status.pinSource, closeReason: status.closeReason) { [weak self] in
            guard let self = self else { return }
            // The transport's disconnect also closes the core's device; it
            // calls `disconnect()` (`link_close`) on this session.
            if let transport = self.transport { transport.disconnect() } else { self.disconnect() }
        }
    }
}

/// Every lost session's banner, over the main window.
struct LinkBannerStack: View {
    @ObservedObject var center: LinkBannerCenter

    var body: some View {
        VStack(spacing: 0) {
            ForEach(center.entries) { entry in
                LinkBannerView(entry: entry) { center.disconnect(entry.id) }
            }
        }
    }
}

/// One lost session: the core's text, the hub and the reason, the way out.
struct LinkBannerView: View {
    let entry: LinkBannerEntry
    let onDisconnect: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "wifi.exclamationmark")
                .foregroundColor(.orange)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.text).font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                if !entry.detail.isEmpty {
                    Text(entry.detail).font(.caption).foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
            ProgressView().controlSize(.small).accessibilityHidden(true)
            Button(entry.action, action: onDisconnect)
                .accessibilityIdentifier("linkBannerDisconnect")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Color.orange.opacity(0.15))
        .overlay(Divider(), alignment: .bottom)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("linkBanner")
        // VoiceOver says it when it appears, as a sighted user sees it at once.
        .onAppear { AccessibilityNotification.Announcement(entry.text).post() }
    }
}
