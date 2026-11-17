//
//  LinkSecurity.swift
//  DSPi Console
//
//  WP SECSW.  The DSPi Link client's side of the security plan
//  (docs/core-merge/PLAN-SEC.md 4.3), on Mac and iOS, as Android's
//  HubSecurity.kt / LinkSecurityUi.kt, Qt's LinkController and the
//  terminal's connect.rs do it:
//
//  - the connection's encryption as the core reports it (`link_security`):
//    encrypted, local (loopback: not red, SECFIX's one rule) or unsecured,
//    for the connection bar (red, " (unsecured)" and an info button, the
//    owner's rule of 2026-11-15; the words are the core's notice rows
//    `dlink.unsecured` and `dlink.unsecured.info`);
//  - the hub list's badge (verified, encrypted, unencrypted);
//  - the codes after which only pairing again is offered (no "connect
//    anyway": nothing was sent to the hub in any of them), and their words;
//  - the questions a hub's session asks the person (`LinkPairingUI`): the
//    PIN with the hub key's fingerprint, pairing again, a `dspi://pair` link;
//  - `dspi://pair` links, parsed by the core (`link_parse_pair_link`).
//
//  Main thread, as CoreLink.
//

import Foundation
import SwiftUI
import Combine
import dspi_core

// MARK: - The connection's encryption

/// What the UI shows about a hub connection's encryption.
enum HubConnectionSecurity: Equatable {
    /// TLS with the hub key checked (its fingerprint).
    case encrypted(hubKey: String)
    /// Not encrypted, but the hub is this machine (loopback): nothing crosses
    /// a network, so nothing is shown.
    case local
    /// Not encrypted: a hub without TLS, an upstream hub, a hub in
    /// compatibility mode.  The bar is red and says " (unsecured)".
    case unsecured

    /// The core's `link_security()` of a live session.
    init(_ s: dspi_core.LinkSecurity) {
        if s.encrypted { self = .encrypted(hubKey: s.hubKey) }
        else if s.local { self = .local }
        else { self = .unsecured }
    }

    var isUnsecured: Bool { self == .unsecured }
}

/// The shared words of the unsecured bar: the core's notice table, every
/// client the same.
enum LinkSecurityWording {
    /// The bar's row: its title is the suffix's word.
    static let unsecuredKey = "dlink.unsecured"
    /// The explanation the info button opens (calm: what it means, how to
    /// turn TLS on).
    static let infoKey = "dlink.unsecured.info"

    /// " (unsecured)", appended to the bar's text.
    static var suffix: String { " (\(ConsoleNotice.row(unsecuredKey).title))" }

    /// The info button's explanation.
    static var info: ConsoleNotice { .row(infoKey) }

    /// The bar's text for `security` (nil: not a hub's device).
    static func barText(_ text: String, _ security: HubConnectionSecurity?) -> String {
        security?.isUnsecured == true ? text + suffix : text
    }

    // -- WP SECSWF --
    /// The iOS toolbar's Devices button for the selected device's session
    /// (`connected` false: no session to judge): "Devices (unsecured)" and red
    /// when it is unsecured; plain otherwise, loopback (`local`) included.
    static func devicesButton(_ security: HubConnectionSecurity?, connected: Bool) -> (title: String, unsecured: Bool) {
        let unsecured = connected && security?.isUnsecured == true
        return (unsecured ? "Devices" + suffix : "Devices", unsecured)
    }
}

/// Each hub session's encryption, by the composite's hub key
/// (`DiscoveredHub.id`), fed by its CoreLink on every look at the core.  A
/// hub with no live session has no entry.  Main thread.
final class LinkSecurityCenter: ObservableObject {
    static let shared = LinkSecurityCenter()

    @Published private(set) var byHub: [String: HubConnectionSecurity] = [:]

    func set(_ hubID: String, _ security: HubConnectionSecurity?) {
        if byHub[hubID] != security { byHub[hubID] = security }
    }

    /// The encryption of `device`'s session (nil: not a hub's device, or no
    /// session yet).
    func security(of device: DSPiDevice?) -> HubConnectionSecurity? {
        guard let hub = device?.hub else { return nil }
        return byHub[hub.hubID]
    }
}

/// The small info button beside "(unsecured)": opens the core's calm
/// explanation.  Accessible by its label, not by its colour.
struct UnsecuredInfoButton: View {
    @State private var open = false

    static let label = "Why unsecured?"

    var body: some View {
        Button { open = true } label: {
            Image(systemName: "info.circle")
        }
        .buttonStyle(.borderless)
        .foregroundColor(.red)
        .accessibilityLabel(Self.label)
        .accessibilityIdentifier("unsecuredInfo")
        .help(Self.label)
        .popover(isPresented: $open) {
            let n = LinkSecurityWording.info
            VStack(alignment: .leading, spacing: 8) {
                Text(n.title).font(.headline)
                Text(n.reason).fixedSize(horizontal: false, vertical: true)
                Text(n.action).font(.caption).foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding()
            .frame(width: 320)
            .accessibilityIdentifier("unsecuredInfoText")
            .presentationCompactAdaptation(.popover)   // -- WP SECSWF -- a popover on the iPhone too
        }
    }
}

// MARK: - The hub list's badge

/// A hub list row's encryption badge.
enum HubSecurityBadge: Equatable {
    /// Paired, and its key is pinned: every connection checks it.
    case verified
    /// Not paired over TLS yet, but its record says it serves TLS.
    case encrypted
    /// Neither: an upstream hub, TLS off, or paired before TLS and not met since.
    case unencrypted
    /// A typed address with nothing known yet.
    case unknown

    var label: String {
        switch self {
        case .verified: return "Verified"
        case .encrypted: return "Encrypted"
        case .unencrypted: return "Unencrypted"
        case .unknown: return ""
        }
    }

    var icon: String {
        switch self {
        case .verified: return "checkmark.shield"
        case .encrypted: return "lock.shield"
        case .unencrypted, .unknown: return "exclamationmark.shield"
        }
    }

    var color: Color {
        switch self {
        case .verified, .encrypted: return .green
        case .unencrypted, .unknown: return .orange
        }
    }

    /// The badge of `hub` with its stored credential.  The advert is a hint
    /// only; the pin (`link_credential_hub_key`) is what the core checks.
    static func of(_ hub: DiscoveredHub, credential: String?) -> HubSecurityBadge {
        if let c = credential, !linkCredentialHubKey(credential: c).isEmpty { return .verified }
        if hub.tls { return .encrypted }
        return hub.isManual ? .unknown : .unencrypted
    }

    /// The badge as the hub lists show it, from the app's stored credentials.
    static func of(_ hub: DiscoveredHub, tokens: LinkTokenStore) -> HubSecurityBadge {
        let id = CoreLink.expectedID(of: hub)
        return of(hub, credential: (id.isEmpty ? nil : tokens.token(forHub: id)) ?? tokens.token(forHub: hub.id))
    }
}

/// A hub list row's badge view (nothing for `.unknown`).
struct HubSecurityBadgeView: View {
    let badge: HubSecurityBadge

    var body: some View {
        if badge != .unknown {
            Label(badge.label, systemImage: badge.icon)
                .font(.caption.weight(.medium))
                .foregroundStyle(badge.color)
                .accessibilityIdentifier("hubSecurityBadge")
        }
    }
}

// -- WP SWPIN -- (review-pintls N2)
/// A stored record that is a pairing: it carries a token (a `v1` token, or a
/// `v2` credential with one).  The core's unpaired record (a hub pinned
/// before: TLS only, no token) is not, so such a hub still asks for its PIN
/// and is never picked on its own at launch.  -- WP PINX -- (review-fu1 N1)
/// The core says so (`linkCredentialHasToken`): a damaged `v2` record, or a
/// pinned one with an empty token, is not a pairing either.
enum LinkRecord {
    static func isPaired(_ record: String?) -> Bool {
        guard let record = record, !record.isEmpty else { return false }
        return linkCredentialHasToken(credential: record)
    }
}

// MARK: - Pairing again

/// The `DspiError.Link` codes after which the app offers pairing again, and
/// the only ones (PLAN-SEC 4.3): nothing was sent to the hub, and there is no
/// "connect anyway".
///
/// -- WP PINTLS -- (owner decision 2026-11-17) Pairing again keeps a hub that
/// was pinned TLS-only (the core's `linkCredentialUnpaired` record): it pairs
/// over TLS only.  When such a hub refuses TLS (`tls_required_pinned`) the
/// one offer is Forget Hub, which deletes its pairing and key; that is the
/// only way back to an unencrypted connection, and the text says so.
enum LinkRepair {
    static let codes: Set<String> = ["hub_key_changed", "hub_key_unverified", "tls_refused", "tls_required",
                                     forgetCode]
    /// -- WP PINTLS -- the code whose only offer is Forget Hub.
    static let forgetCode = "tls_required_pinned"

    /// The code of `error` when it is one of `codes`.
    static func code(of error: DspiError) -> String? {
        if case .Link(let code, _) = error, codes.contains(code) { return code }
        return nil
    }

    /// The code of a session the core ended for good (`.refused`): a
    /// plaintext one the hub closed with 4003 `tls_required`, or a reconnect
    /// that met another key.  `LinkStatus` carries the core's words only
    /// (dspi-link's `LinkError` Display), so they are matched here.
    static func code(ofCloseReason reason: String) -> String? {
        if reason.hasPrefix("tls_required") { return "tls_required" }
        if reason.hasPrefix("this hub's key changed") { return "hub_key_changed" }
        if reason.hasPrefix("this hub's key does not match") { return "hub_key_unverified" }
        if reason.hasPrefix("this hub was paired with encryption before") { return forgetCode }   // -- WP PINTLS --
        return nil
    }

    /// Why pairing again is offered, by the core's code; nothing was sent to
    /// the hub in any of them.
    static func text(code: String, hubName: String, fromPairLink: Bool) -> String {
        switch code {
        case "hub_key_changed":
            return "\(hubName) presented another key than the one it had when this console paired with it, "
                + "so nothing was sent to it. If the hub was set up again or its key was reset, pair again "
                + "and check that the key matches the one the hub shows."
        case "hub_key_unverified":
            return fromPairLink
                ? "\(hubName) did not present the key in the pairing link, so no PIN was sent. The link may "
                    + "be old or not from this hub. Connect to the hub as usual instead."
                : "\(hubName) did not present the key it was expected to have, so nothing was stored. Pair "
                    + "again and check the key the hub shows."
        case "tls_refused":
            return "\(hubName) was set up with encryption, or says it supports it, but refused an encrypted "
                + "connection. Nothing was sent unencrypted. Check the hub's encryption setting, then pair again. "
                + "Pairing again uses an encrypted connection only."
        case forgetCode:   // -- WP PINTLS --
            return "\(hubName) was paired with encryption before, so this console connects to it only encrypted, "
                + "and it refused encryption. Nothing was sent, not even the PIN. Turn TLS on in the hub's settings "
                + "if you can. Forget Hub deletes its pairing and key; it can then be paired without encryption, "
                + "and on this network the PIN and settings could be read or changed."
        default:
            return "\(hubName) accepts only encrypted connections. Pair again to set one up."
        }
    }

    /// The button that pairs again.
    static func actionTitle(code: String, fromPairLink: Bool) -> String {
        if code == forgetCode { return "Forget Hub" }   // -- WP PINTLS --
        return fromPairLink && code == "hub_key_unverified" ? "Connect" : "Pair Again"
    }
}

// MARK: - The person's questions

/// A hub asks for its PIN.
struct LinkPairingAsk: Equatable {
    let hubName: String
    /// The fingerprint of the key the hub presented (`SHA256:K7Q2-...`), ""
    /// on an unencrypted connection.
    let hubKey: String
    /// The PIN a pairing link carried, filled in; pairing still needs the tap.
    let pin: String
}

/// A dial ended with one of `LinkRepair.codes`.
struct LinkRepairAsk: Equatable {
    let hubName: String
    let code: String
    let text: String
    let actionTitle: String
}

/// The questions a hub's session asks the person: the Mac's alerts
/// (PairingPrompt.swift), the iOS sheets (IOSLinkPairing.swift).
protocol LinkPairingUI: AnyObject {
    /// The PIN, or nil to give up.
    func askPIN(_ ask: LinkPairingAsk, completion: @escaping (String?) -> Void)
    /// Pair again (true) or leave the hub (false).
    func askRepair(_ ask: LinkRepairAsk, completion: @escaping (Bool) -> Void)
    /// Connect to the hub a `dspi://pair` link names (true), after the person
    /// has seen its name, address and key.
    func confirmPairLink(_ link: LinkPairLink, completion: @escaping (Bool) -> Void)
}

// MARK: - Pairing links

/// `dspi://pair?h=<hid>&a=<host:port>&k=<key>&p=<pin>`, the hub's QR code: a
/// hint, never a bypass.  The core parses it; nothing is dialled before the
/// person's tap, and the hub must present the link's key before any PIN goes
/// out (`hub_key_unverified` otherwise).
enum LinkPairLinks {
    /// The scheme the app registers (Info.plist `CFBundleURLTypes`).
    static let scheme = "dspi"

    /// Whether `url` is meant for this handler (a `dspi:` URL).
    static func handles(_ url: URL) -> Bool { url.scheme?.lowercased() == scheme }

    /// Why a link was refused, in words to show.
    struct Refusal: Error, Equatable {
        let message: String
    }

    /// The core's parse of `url`, or the reason it is refused.
    static func parse(_ url: URL) -> Result<LinkPairLink, Refusal> {
        do {
            return .success(try linkParsePairLink(text: url.absoluteString))
        } catch let error as DspiError {
            if case .InvalidArgument(let msg) = error { return .failure(Refusal(message: "Not a valid pairing link: \(msg).")) }
            return .failure(Refusal(message: ConsoleNotice.of(error).line))
        } catch {
            return .failure(Refusal(message: "Not a valid pairing link."))
        }
    }

    /// The address as the person reads it.
    static func address(_ link: LinkPairLink) -> String {
        link.host.contains(":") ? "[\(link.host)]:\(link.port)" : "\(link.host):\(link.port)"
    }

    /// What the confirmation says.
    static func confirmText(_ link: LinkPairLink) -> String {
        "Connect to the DSPi Link hub at \(address(link)) (hub id \(link.hubId))? It must present this key:\n"
            + "\(link.hubKey)\nCheck that it matches the key the hub shows. Nothing is sent before it does."
    }

    /// A link that arrived (a scanned QR code, a tap in another app): parsed,
    /// confirmed by the person, then dialled through the composite.  A link
    /// that does not parse is refused with a notice and nothing else.
    @MainActor
    static func open(_ url: URL, ui: LinkPairingUI?, coordinator: LinkClientCoordinator,
                     notices: NoticeCenter = .main) {
        switch parse(url) {
        case .failure(let why):
            notices.post(.result(why.message, kind: .error))
        case .success(let link):
            guard let ui = ui else {
                notices.post(.result("Pairing links cannot be confirmed here.", kind: .error))
                return
            }
            ui.confirmPairLink(link) { yes in
                Task { @MainActor in
                    guard yes else { return }
                    if let why = coordinator.connect(pairLink: link) { notices.post(.result(why, kind: .error)) }
                }
            }
        }
    }
}
