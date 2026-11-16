//
//  HubSecuritySettingsView.swift
//  DSPi Console
//
//  -- WP P2SW -- The Mac hub's security and services on the Networking page
//  (review batch 6 F5, PLAN-P2 P2SW): the TLS mode the hub serves (required
//  by default, as `uconsole hub` and the Qt app), its key's fingerprint, the
//  hub page's https address, the pairing link and its QR code under the PIN,
//  the owner-approved upstream compatibility mode (off by default; turning
//  it on asks first, with the large warning listing every effect from the
//  hub's own list, as Qt does), and the optional services, every one off by
//  default.  The rules are the hub's (CoreHubService, dspi_hub).
//

#if os(macOS)
import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins

/// The warning's words.
enum HubSecurityWords {
    /// The warning's body: every effect, one line each.
    static func effects(_ list: [String]) -> String {
        list.map { (effect: String) -> String in
            let first = String(effect.prefix(1)).uppercased()
            let rest = String(effect.dropFirst())
            return "\u{2022} \(first)\(rest)."
        }.joined(separator: "\n")
    }

    static let compatTitle = "Security downgraded: upstream compatibility mode"
}

/// The hub's security and its services, as two Form sections.
struct HubSecuritySections: View {
    @ObservedObject var hub: CoreHubService

    var body: some View {
        securitySection
        servicesSection
    }

    private var securitySection: some View {
        Section {
            LabeledContent("Encryption") {
                Text(hub.tlsWords)
                    .foregroundColor(hub.servesCompat ? .red : .secondary)
                    .accessibilityIdentifier("hubTlsMode")
            }
            if let fp = hub.fingerprint {
                LabeledContent("Key fingerprint") {
                    Text(verbatim: fp)
                        .font(.system(.callout, design: .monospaced))
                        .textSelection(.enabled)
                        .accessibilityIdentifier("hubFingerprint")
                }
            }
            if let page = hub.hubPageURL, let url = URL(string: page) {
                LabeledContent("Hub page") {
                    Link(page, destination: url).accessibilityIdentifier("hubPageLink")
                }
            }
            Toggle(isOn: Binding(get: { hub.upstreamCompat || hub.compatPending },
                                 set: { hub.setUpstreamCompat($0) })) {
                Text("Upstream compatibility mode")
                Text("For DSPi Link clients that cannot encrypt. Off unless you need it.")
            }
            .accessibilityIdentifier("hubUpstreamCompat")
            if hub.upstreamCompat {
                CompatWarningBox(effects: CoreHubService.compatEffects)
            }
        } header: {
            Text("Security")
        } footer: {
            Text("Clients check this fingerprint when they pair, and refuse a hub whose key changed. The hub page uses the hub's own certificate: your browser asks once whether to trust it.")
                .font(.caption).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }
        .sheet(isPresented: Binding(get: { hub.compatPending }, set: { if !$0 { hub.cancelUpstreamCompat() } })) {
            CompatConfirmSheet(effects: CoreHubService.compatEffects,
                               confirm: { hub.confirmUpstreamCompat() },
                               cancel: { hub.cancelUpstreamCompat() })
        }
    }

    private var servicesSection: some View {
        Section {
            ForEach(CoreHubService.OptionalService.allCases) { service in
                Toggle(isOn: Binding(get: { hub.isOn(service) }, set: { hub.set(service, $0) })) {
                    Text(service.title)
                    Text(service.detail)
                }
                .accessibilityIdentifier("hubService.\(service.rawValue)")
            }
        } header: {
            Text("Hub Services")
        } footer: {
            Text("Every service is off until you turn it on. A change restarts sharing.")
                .font(.caption).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// While the compatibility mode is on: what it turns off, in red.
struct CompatWarningBox: View {
    let effects: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(HubSecurityWords.compatTitle, systemImage: "exclamationmark.shield.fill")
                .font(.callout.weight(.semibold))
            Text(HubSecurityWords.effects(effects)).font(.caption)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(.red)
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("hubCompatWarning")
    }
}

/// The question before the compatibility mode goes on: the large warning.
struct CompatConfirmSheet: View {
    let effects: [String]
    let confirm: () -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: "exclamationmark.shield.fill")
                    .font(.system(size: 40)).foregroundStyle(.red)
                Text("Turn on upstream compatibility mode?")
                    .font(.title2.weight(.bold))
            }
            Text("Security is downgraded while it is on. These protections turn off:")
                .font(.headline)
            Text(HubSecurityWords.effects(effects))
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("hubCompatEffects")
            Text("Use it only on a network you trust, and only while you need it.")
                .font(.callout.weight(.semibold)).foregroundStyle(.red)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel, action: cancel)
                    .keyboardShortcut(.cancelAction)
                Button("Turn On", role: .destructive, action: confirm)
                    .accessibilityIdentifier("hubCompatConfirm")
            }
        }
        .padding(24)
        .frame(width: 520)
    }
}

/// Under the PIN: the pairing link and its QR code, for a client to pair and
/// pin the hub's key in one step.
struct PairingLinkRow: View {
    @ObservedObject var hub: CoreHubService

    var body: some View {
        if let link = hub.pairingLink {
            VStack(spacing: 6) {
                if let image = Self.qr(link) {
                    Image(nsImage: image)
                        .interpolation(.none)
                        .resizable()
                        .frame(width: 160, height: 160)
                        .accessibilityLabel("Pairing QR code")
                        .accessibilityIdentifier("hubPairingQR")
                }
                Text(verbatim: link)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("hubPairingLink")
                Text("Scan it on the new client, or open the link there. It carries the PIN: show it only to that client.")
                    .font(.caption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
        }
    }

    /// The QR code of `text`, crisp at any size.
    static func qr(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let out = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
        let rep = NSCIImageRep(ciImage: out)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}
#endif
