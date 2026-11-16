//
//  IOSLinkPairing.swift
//  DSPi Console (iOS target)
//
//  Pairing with a DSPi Link hub that asks for a PIN, as a sheet (work
//  package A7).  The Mac asks with a modal NSAlert through
//  `CompositeTransport.pairingPrompt`; iOS cannot block the main thread for a
//  sheet, so it answers `asyncPairingPrompt` instead: the request is published
//  here, a sheet shows it, and the PIN (or nil for Cancel) goes back to the
//  hub's NetworkTransport.  A wrong PIN makes the transport ask again, which
//  shows the sheet again.
//
//  -- WP SECSW -- The iOS `LinkPairingUI`: the sheet shows the key the hub
//  presented (and a pairing link's PIN filled in), and two alerts ask about
//  pairing again (LinkRepair) and a `dspi://pair` link (LinkPairLinks).
//

#if os(iOS)
import SwiftUI
import dspi_core

@MainActor
final class IOSLinkPairing: ObservableObject {
    static let shared = IOSLinkPairing()

    struct Request: Identifiable {
        let id = UUID()
        let hubName: String
        /// -- WP SECSW -- The key the hub presented ("" unencrypted).
        var hubKey: String = ""
        /// -- WP SECSW -- A pairing link's PIN, filled in.
        var pin: String = ""
        fileprivate let completion: (String?) -> Void
    }

    @Published var request: Request?

    /// -- WP SECSW -- A yes-or-no question: pairing again, a pairing link.
    struct Question: Identifiable {
        let id = UUID()
        let title: String
        let text: String
        let action: String
        fileprivate let completion: (Bool) -> Void
    }

    @Published var question: Question?

    /// `CompositeTransport.asyncPairingPrompt`.
    nonisolated static func ask(hubName: String, completion: @escaping (String?) -> Void) {
        Task { @MainActor in shared.present(Request(hubName: hubName, completion: completion)) }
    }

    fileprivate func present(_ new: Request) {
        // One question at a time: an older one is given up.
        request?.completion(nil)
        request = new
    }

    fileprivate func present(_ new: Question) {
        question?.completion(false)
        question = new
    }

    /// -- WP SECSW -- The person's answer to `question`.
    func answerQuestion(_ yes: Bool) {
        guard let pending = question else { return }
        question = nil
        pending.completion(yes)
    }

    /// The user's answer: a PIN, or nil for Cancel.
    func answer(_ pin: String?) {
        guard let pending = request else { return }
        request = nil
        pending.completion(pin)
    }

    /// Valid PINs are six digits (DSPi Link spec 7.2).
    nonisolated static func isValid(_ pin: String) -> Bool {
        pin.count == 6 && pin.allSatisfy(\.isNumber)
    }
}

// -- WP SECSW --
/// The iOS `LinkPairingUI`: each question goes to `IOSLinkPairing.shared`'s
/// sheet or alert on the main actor.
final class IOSLinkPairingUI: LinkPairingUI {
    func askPIN(_ ask: LinkPairingAsk, completion: @escaping (String?) -> Void) {
        Task { @MainActor in
            IOSLinkPairing.shared.present(IOSLinkPairing.Request(hubName: ask.hubName, hubKey: ask.hubKey,
                                                                 pin: ask.pin, completion: completion))
        }
    }

    func askRepair(_ ask: LinkRepairAsk, completion: @escaping (Bool) -> Void) {
        Task { @MainActor in
            IOSLinkPairing.shared.present(IOSLinkPairing.Question(title: "Could not connect securely", text: ask.text,
                                                                  action: ask.actionTitle, completion: completion))
        }
    }

    func confirmPairLink(_ link: LinkPairLink, completion: @escaping (Bool) -> Void) {
        Task { @MainActor in
            IOSLinkPairing.shared.present(IOSLinkPairing.Question(title: "Pair with a DSPi Link hub?",
                                                                  text: LinkPairLinks.confirmText(link),
                                                                  action: "Connect", completion: completion))
        }
    }
}

struct IOSPairingSheet: View {
    let request: IOSLinkPairing.Request
    @State private var pin = ""
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("000000", text: $pin)
                        .keyboardType(.numberPad)
                        .textContentType(.oneTimeCode)
                        .font(.system(size: 30, weight: .medium, design: .monospaced))
                        .multilineTextAlignment(.center)
                        .focused($focused)
                        .accessibilityLabel("Pairing code")
                        .onChange(of: pin) { _, value in
                            let digits = String(value.filter(\.isNumber).prefix(6))
                            if digits != value { pin = digits }
                        }
                } header: {
                    Text("Pairing code")
                } footer: {
                    Text("""
                        On a Mac sharing its DSPi with uConsole for DSPi: open Settings, Networking, \
                        choose Allow a New Client and enter the code it shows within two minutes. \
                        On a DSPi Link bridge: enter the PIN set on its web page.
                        """)
                }
                // -- WP SECSW -- the key to compare with the one the hub shows
                Section {
                    if request.hubKey.isEmpty {
                        Text("This connection is not encrypted.")
                            .foregroundStyle(.red)
                            .accessibilityIdentifier("pairingHubKey")
                    } else {
                        Text(request.hubKey)
                            .font(.system(.body, design: .monospaced))
                            .textSelection(.enabled)
                            .accessibilityIdentifier("pairingHubKey")
                    }
                } header: {
                    Text("Hub key")
                } footer: {
                    if !request.hubKey.isEmpty {
                        Text("Check that it matches the key the hub shows before you pair.")
                    }
                }
            }
            .navigationTitle("Pair with \(request.hubName)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { IOSLinkPairing.shared.answer(nil) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Pair") { IOSLinkPairing.shared.answer(pin) }
                        .disabled(!IOSLinkPairing.isValid(pin))
                }
            }
            .onAppear {
                if pin.isEmpty, !request.pin.isEmpty { pin = String(request.pin.prefix(6)) }   // -- WP SECSW --
                focused = true
            }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled()
        .preferredColorScheme(.dark)
    }
}

extension View {
    /// Presents the pairing sheet when a hub asks for a PIN.  `active` false
    /// leaves it to another presenter (the Devices sheet, while it is up).
    func linkPairingSheet(active: Bool = true) -> some View {
        modifier(LinkPairingSheetModifier(active: active))
    }
}

private struct LinkPairingSheetModifier: ViewModifier {
    let active: Bool
    @ObservedObject private var pairing = IOSLinkPairing.shared

    func body(content: Content) -> some View {
        content.sheet(item: Binding(get: { active ? pairing.request : nil },
                                    set: { if $0 == nil, active { pairing.answer(nil) } })) { request in
            IOSPairingSheet(request: request)
        }
        // -- WP SECSW -- pairing again, a pairing link
        .alert(pairing.question?.title ?? "",
               isPresented: Binding(get: { active && pairing.question != nil },
                                    set: { _ in }),   // each button answers
               presenting: pairing.question) { q in
            Button(q.action) { pairing.answerQuestion(true) }
            Button("Cancel", role: .cancel) { pairing.answerQuestion(false) }
        } message: { q in
            Text(q.text)
        }
    }
}
#endif
