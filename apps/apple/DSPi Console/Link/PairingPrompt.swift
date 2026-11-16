//
//  PairingPrompt.swift
//  DSPi Console
//
//  The modal that asks for a hub's pairing code, in the app's NSAlert style.
//  -- WP SECSW -- With the hub key's fingerprint, and the Mac's
//  `LinkPairingUI`: the PIN, pairing again, a `dspi://pair` link.
//

import AppKit
import dspi_core

enum PairingPrompt {
    /// Ask for the six-digit code the hub is showing.  Main thread only.
    static func ask(hubName: String) -> String? {
        ask(LinkPairingAsk(hubName: hubName, hubKey: "", pin: ""))
    }

    /// -- WP SECSW -- The same, with the key the hub presented (to compare
    /// with the one the hub shows) and a pairing link's PIN filled in.
    static func ask(_ ask: LinkPairingAsk) -> String? {
        let alert = NSAlert()
        alert.messageText = "Pair with \(ask.hubName)"
        alert.informativeText = informativeText(ask)
        alert.alertStyle = .informational
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 160, height: 24))
        field.placeholderString = "000000"
        field.alignment = .center
        field.font = .monospacedDigitSystemFont(ofSize: 18, weight: .medium)
        field.stringValue = ask.pin
        alert.accessoryView = field
        alert.addButton(withTitle: "Pair")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        let pin = field.stringValue.trimmingCharacters(in: .whitespaces)
        return pin.isEmpty ? nil : pin
    }

    /// -- WP SECSW -- The alert's text: where the code is, and the key.
    static func informativeText(_ ask: LinkPairingAsk) -> String {
        var text = "On \(ask.hubName), open Settings > Networking and choose Allow a New Client, then enter the code it shows."
        if ask.hubKey.isEmpty {
            text += "\n\nThis connection is not encrypted."
        } else {
            text += "\n\nHub key: \(ask.hubKey)\nCheck that it matches the key the hub shows before you pair."
        }
        return text
    }
}

/// -- WP SECSW -- The Mac's questions: modal alerts, answered at once.
final class MacLinkPairingUI: LinkPairingUI {
    func askPIN(_ ask: LinkPairingAsk, completion: @escaping (String?) -> Void) {
        completion(PairingPrompt.ask(ask))
    }

    func askRepair(_ ask: LinkRepairAsk, completion: @escaping (Bool) -> Void) {
        let alert = NSAlert()
        alert.messageText = "Could not connect securely"
        alert.informativeText = ask.text
        alert.alertStyle = .warning
        alert.addButton(withTitle: ask.actionTitle)
        alert.addButton(withTitle: "Cancel")
        completion(alert.runModal() == .alertFirstButtonReturn)
    }

    func confirmPairLink(_ link: LinkPairLink, completion: @escaping (Bool) -> Void) {
        let alert = NSAlert()
        alert.messageText = "Pair with a DSPi Link hub?"
        alert.informativeText = LinkPairLinks.confirmText(link)
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Connect")
        alert.addButton(withTitle: "Cancel")
        completion(alert.runModal() == .alertFirstButtonReturn)
    }
}
