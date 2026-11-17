//
//  LinkSecuritySECSWTests.swift
//  DSPi ConsoleTests
//
//  WP SECSW.  The client side of the security plan (PLAN-SEC 4.3) on Mac and
//  iOS: the Keychain item's attributes, the v2 credential, the connection
//  bar's three states and the owner's " (unsecured)" words, the hub list's
//  badge, the pair-again codes, `dspi://pair` links through the core's parser
//  (a wrong `k=` refused before anything), the `-CoreTransport NO` fallback
//  refusing a hub whose record says TLS is required, and SECFWV's label.
//  macOS also runs CoreLink against the Mac's Swift hub in process (plaintext,
//  loopback): a pairing link whose key the hub does not present is refused
//  with no PIN prompt, a pinned credential to a hub without TLS offers only
//  pairing again, and pairing again pairs.
//

import XCTest
import Combine
import Security
import dspi_core
@testable import DSPi_Console

// MARK: - Helpers

@discardableResult
private func secswWait(_ timeout: TimeInterval, _ what: String = "", file: StaticString = #filePath, line: UInt = #line,
                       _ condition: () -> Bool) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while !condition() {
        if Date() > deadline {
            if !what.isEmpty { XCTFail("timed out waiting for \(what)", file: file, line: line) }
            return false
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    }
    return true
}

private func secswTokens() -> LinkTokenStore { LinkTokenStore(service: "secsw-test-\(UUID().uuidString)") }

/// 32 zero bytes as base64url: a well-formed key no hub has.
private let secswForeignKey = String(repeating: "A", count: 43)

private func secswLink(hub: String = "abcdef01-0000-4000-8000-00000000c0de", address: String = "127.0.0.1:11915",
                       key: String = secswForeignKey, pin: String = "") throws -> LinkPairLink {
    try linkParsePairLink(text: "dspi://pair?h=\(hub)&a=\(address)&k=\(key)" + (pin.isEmpty ? "" : "&p=\(pin)"))
}

/// The person's answers, recorded.
final class SECSWFakeUI: LinkPairingUI {
    var pinAsks: [LinkPairingAsk] = []
    var pins: [String?] = []
    var repairs: [LinkRepairAsk] = []
    var repairAnswers: [Bool] = []
    var links: [LinkPairLink] = []
    var linkAnswer = false

    func askPIN(_ ask: LinkPairingAsk, completion: @escaping (String?) -> Void) {
        pinAsks.append(ask)
        completion(pins.isEmpty ? nil : pins.removeFirst())
    }

    func askRepair(_ ask: LinkRepairAsk, completion: @escaping (Bool) -> Void) {
        repairs.append(ask)
        completion(repairAnswers.isEmpty ? false : repairAnswers.removeFirst())
    }

    func confirmPairLink(_ link: LinkPairLink, completion: @escaping (Bool) -> Void) {
        links.append(link)
        completion(linkAnswer)
    }
}

// MARK: - Both targets

final class LinkSecuritySECSWTests: XCTestCase {

    // MARK: The Keychain

    /// The item is AfterFirstUnlockThisDeviceOnly and never synchronizable.
    /// iOS (and a Mac build with an application identifier) uses the
    /// data-protection keychain, where the class is asserted; an ad-hoc signed
    /// Mac build keeps the login keychain, which has no class and never syncs.
    func testSECSWTheKeychainItemIsThisDeviceOnlyAndNotSynced() throws {
        let tokens = secswTokens()
        defer { tokens.removeToken(forHub: "hub-a") }
        tokens.setToken("tok-1", forHub: "hub-a")
        let attrs = try XCTUnwrap(tokens.attributes(forHub: "hub-a"), "the item was written")
        let sync = attrs[kSecAttrSynchronizable as String] as? NSNumber
        XCTAssertFalse(sync?.boolValue ?? false, "never synchronizable: \(attrs)")
        #if os(iOS)
        XCTAssertEqual(tokens.backend, .dataProtection)
        #endif
        print("SECSW: keychain backend \(tokens.backend), attributes \(attrs.keys.sorted())")
        if tokens.backend == .dataProtection {
            XCTAssertEqual(attrs[kSecAttrAccessible as String] as? String, LinkTokenStore.accessibility,
                           "kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly")
        }
        // An update keeps the class.
        tokens.setToken("tok-2", forHub: "hub-a")
        XCTAssertEqual(tokens.token(forHub: "hub-a"), "tok-2")
        if tokens.backend == .dataProtection {
            XCTAssertEqual(tokens.attributes(forHub: "hub-a")?[kSecAttrAccessible as String] as? String,
                           LinkTokenStore.accessibility)
        }
    }

    /// An item written with another class (before this WP) is moved to
    /// AfterFirstUnlockThisDeviceOnly when it is read.
    func testSECSWAnOlderItemIsMovedToTheClassOnRead() throws {
        let tokens = secswTokens()
        guard tokens.backend == .dataProtection else {
            throw XCTSkip("the login keychain has no protection class (an ad-hoc signed Mac build)")
        }
        let service = "secsw-old-\(UUID().uuidString)"
        var add: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                  kSecAttrService as String: service,
                                  kSecAttrAccount as String: "hub-b",
                                  kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlocked as String,
                                  kSecValueData as String: Data("old".utf8)]
        #if os(macOS)
        add[kSecUseDataProtectionKeychain as String] = true
        #endif
        XCTAssertEqual(SecItemAdd(add as CFDictionary, nil), errSecSuccess)
        let store = LinkTokenStore(service: service)
        defer { store.removeToken(forHub: "hub-b") }
        XCTAssertEqual(store.attributes(forHub: "hub-b")?[kSecAttrAccessible as String] as? String,
                       kSecAttrAccessibleWhenUnlocked as String, "written with the old class")
        XCTAssertEqual(store.token(forHub: "hub-b"), "old")
        XCTAssertEqual(store.attributes(forHub: "hub-b")?[kSecAttrAccessible as String] as? String,
                       LinkTokenStore.accessibility, "moved on read")
    }

    /// A v2 credential (the core's string with the key pin) round-trips, and
    /// the core reads its pin back.
    func testSECSWAV2CredentialRoundTrips() throws {
        let link = try secswLink()
        XCTAssertTrue(link.credential.hasPrefix("v2."), link.credential)
        let tokens = secswTokens()
        defer { tokens.removeToken(forHub: link.hubId) }
        tokens.setToken(link.credential, forHub: link.hubId)
        let back = try XCTUnwrap(tokens.token(forHub: link.hubId))
        XCTAssertEqual(back, link.credential)
        XCTAssertEqual(linkCredentialHubKey(credential: back), link.hubKey)
        XCTAssertTrue(link.hubKey.hasPrefix("SHA256:"), link.hubKey)
        XCTAssertEqual(linkCredentialHubKey(credential: "a-v1-token"), "", "a v1 token has no pin")
    }

    // MARK: The connection bar

    private func security(encrypted: Bool, local: Bool, key: String = "") -> dspi_core.LinkSecurity {
        dspi_core.LinkSecurity(encrypted: encrypted, local: local, hubKey: key, hubKeyNew: false, credential: "")
    }

    /// The three states: encrypted, local (loopback: not red, nothing added)
    /// and unsecured (" (unsecured)", the core's words).
    func testSECSWTheBarsThreeStatesAndTheCoresWords() {
        XCTAssertEqual(HubConnectionSecurity(security(encrypted: true, local: false, key: "SHA256:K7Q2")),
                       .encrypted(hubKey: "SHA256:K7Q2"))
        XCTAssertEqual(HubConnectionSecurity(security(encrypted: false, local: true)), .local)
        XCTAssertEqual(HubConnectionSecurity(security(encrypted: false, local: false)), .unsecured)
        XCTAssertTrue(HubConnectionSecurity.unsecured.isUnsecured)
        XCTAssertFalse(HubConnectionSecurity.local.isUnsecured, "loopback is not red (SECFIX's one rule)")
        XCTAssertFalse(HubConnectionSecurity.encrypted(hubKey: "k").isUnsecured)

        XCTAssertEqual(LinkSecurityWording.suffix, " (unsecured)", "the core's dlink.unsecured title")
        XCTAssertEqual(LinkSecurityWording.barText("Studio via Mac", .unsecured), "Studio via Mac (unsecured)")
        XCTAssertEqual(LinkSecurityWording.barText("Studio via Mac", .local), "Studio via Mac")
        XCTAssertEqual(LinkSecurityWording.barText("Studio via Mac", .encrypted(hubKey: "k")), "Studio via Mac")
        XCTAssertEqual(LinkSecurityWording.barText("DSPi", nil), "DSPi")
        let info = LinkSecurityWording.info
        XCTAssertEqual(info.key, "dlink.unsecured.info")
        XCTAssertTrue(info.reason.contains("not encrypted"), info.reason)
        XCTAssertTrue(info.action.contains("TLS"), info.action)
        XCTAssertEqual(UnsecuredInfoButton.label, "Why unsecured?")
    }

    /// -- WP SECSWF -- The iOS toolbar's Devices button: red and "Devices
    /// (unsecured)" for an unsecured session only; loopback, encrypted, no
    /// hub and no connection are plain.
    func testSECSWFTheDevicesButtonSaysUnsecuredExceptLoopback() {
        let red = LinkSecurityWording.devicesButton(.unsecured, connected: true)
        XCTAssertEqual(red.title, "Devices (unsecured)")
        XCTAssertTrue(red.unsecured)
        for (security, connected) in [(HubConnectionSecurity.local, true), (.encrypted(hubKey: "k"), true),
                                      (nil, true), (.unsecured, false)] as [(HubConnectionSecurity?, Bool)] {
            let plain = LinkSecurityWording.devicesButton(security, connected: connected)
            XCTAssertEqual(plain.title, "Devices", "\(String(describing: security)), connected \(connected)")
            XCTAssertFalse(plain.unsecured)
        }
    }

    /// -- WP SECSWF -- (review N5) A credential read or written once is served
    /// from the process's cache to every store of the service (the badges
    /// make a store per row); a write or a removal goes through to it.
    func testSECSWFCredentialsAreCachedAcrossStores() {
        let service = "secswf-cache-\(UUID().uuidString)"
        let a = LinkTokenStore(service: service), b = LinkTokenStore(service: service)
        defer { a.removeToken(forHub: "hub-c") }
        XCTAssertNil(b.token(forHub: "hub-c"))
        a.setToken("tok-1", forHub: "hub-c")
        XCTAssertEqual(b.token(forHub: "hub-c"), "tok-1", "a write reaches the cache, the cached miss is replaced")
        // Gone from the Keychain behind the store's back: the cache answers.
        SecItemDelete([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
                       kSecAttrAccount as String: "hub-c"] as CFDictionary)
        XCTAssertEqual(b.token(forHub: "hub-c"), "tok-1", "no Keychain read once known")
        a.removeToken(forHub: "hub-c")
        XCTAssertNil(b.token(forHub: "hub-c"))
    }

    /// The center answers for a hub's device, and nothing for a local one.
    func testSECSWTheCenterAnswersForTheSelectedHubDevice() {
        let center = LinkSecurityCenter()
        let remote = DSPiDevice(serial: "S1", locationID: 0,
                                hub: RemoteHubRef(hubID: "hub-1", hubName: "Studio", handle: 1), remoteName: nil)
        let usb = DSPiDevice(serial: "S2", locationID: 1)
        XCTAssertNil(center.security(of: remote))
        center.set("hub-1", .unsecured)
        XCTAssertEqual(center.security(of: remote), .unsecured)
        XCTAssertNil(center.security(of: usb), "a USB device has no hub connection")
        center.set("hub-1", nil)
        XCTAssertNil(center.security(of: remote))
    }

    /// The core's security of a live session: the credential saved when it
    /// changes (a v1 token upgraded to v2 on the first TLS connection), the
    /// bar's state, and the new-key notice once.
    func testSECSWTheCredentialTheCoreReportsIsSavedAndTheKeyNoticeShowsOnce() throws {
        let tokens = secswTokens()
        let id = "abcdef01-0000-4000-8000-00000000beef"
        defer { tokens.removeToken(forHub: id) }
        tokens.setToken("v1-token", forHub: id)
        let center = LinkSecurityCenter()
        let notices = NoticeCenter(infoSeconds: 60)
        let link = CoreLink(hubID: id, hubName: "Studio", expectedHubID: id, tokens: tokens, clientName: "SECSW")
        link.securityCenter = center
        link.notices = notices
        let v2 = try secswLink(hub: id).credential
        var sec = dspi_core.LinkSecurity(encrypted: true, local: false, hubKey: "SHA256:K7Q2-M9XD", hubKeyNew: true,
                                         credential: v2)
        link.applySecurity(sec)
        XCTAssertEqual(tokens.token(forHub: id), v2, "the v1 token upgraded and saved")
        XCTAssertEqual(center.byHub[id], .encrypted(hubKey: "SHA256:K7Q2-M9XD"))
        XCTAssertEqual(notices.last?.title.contains("SHA256:K7Q2-M9XD"), true, "\(String(describing: notices.last))")
        let shown = notices.serial
        link.applySecurity(sec)
        XCTAssertEqual(notices.serial, shown, "the new-key notice shows once")
        sec.credential = ""
        link.applySecurity(sec)
        XCTAssertEqual(tokens.token(forHub: id), v2, "an empty credential never overwrites")
    }

    // MARK: The hub list

    func testSECSWTheHubListBadge() throws {
        let browsed = DiscoveredHub(id: "hub-1", name: "Studio", kind: "console", authMode: "pin", deviceCount: 1,
                                    serials: [], path: "/dspi/v1", tls: false, host: "10.0.0.2", port: 11915,
                                    isManual: false)
        var tls = browsed
        tls.tls = true
        let typed = DiscoveredHub(id: "manual:10.0.0.3:11915", name: "10.0.0.3", kind: "unknown", authMode: "pin",
                                  deviceCount: 0, serials: [], path: "/dspi/v1", tls: false, host: "10.0.0.3",
                                  port: 11915, isManual: true)
        let pinned = try secswLink().credential
        XCTAssertEqual(HubSecurityBadge.of(browsed, credential: pinned), .verified)
        XCTAssertEqual(HubSecurityBadge.of(browsed, credential: "v1-token"), .unencrypted)
        XCTAssertEqual(HubSecurityBadge.of(browsed, credential: nil), .unencrypted)
        XCTAssertEqual(HubSecurityBadge.of(tls, credential: nil), .encrypted, "the advert, a hint")
        XCTAssertEqual(HubSecurityBadge.of(typed, credential: nil), .unknown)
        XCTAssertEqual(HubSecurityBadge.verified.label, "Verified")
        XCTAssertEqual(HubSecurityBadge.unencrypted.label, "Unencrypted")
    }

    // MARK: Pairing again

    func testSECSWOnlyTheKeyAndTLSCodesOfferPairingAgain() {
        XCTAssertEqual(LinkRepair.codes, ["hub_key_changed", "hub_key_unverified", "tls_refused", "tls_required",
                                          "tls_required_pinned"])   // -- WP PINTLS --
        XCTAssertEqual(LinkRepair.code(of: .Link(code: "hub_key_changed", msg: "x")), "hub_key_changed")
        XCTAssertNil(LinkRepair.code(of: .Link(code: "wrong_pin", msg: "x")))
        XCTAssertNil(LinkRepair.code(of: .Disconnected))
        XCTAssertEqual(LinkRepair.code(ofCloseReason: "tls_required: this hub accepts encrypted connections only"),
                       "tls_required")
        XCTAssertEqual(LinkRepair.code(ofCloseReason: "this hub's key changed; pair again to trust the new key"),
                       "hub_key_changed")
        XCTAssertNil(LinkRepair.code(ofCloseReason: "revoked"))
        for code in LinkRepair.codes {
            let text = LinkRepair.text(code: code, hubName: "Studio", fromPairLink: false)
            XCTAssertTrue(text.hasPrefix("Studio"), text)
            XCTAssertFalse(text.lowercased().contains("anyway"), "no connect anyway: \(text)")
        }
        XCTAssertEqual(LinkRepair.actionTitle(code: "hub_key_changed", fromPairLink: false), "Pair Again")
        XCTAssertEqual(LinkRepair.actionTitle(code: "hub_key_unverified", fromPairLink: true), "Connect")
    }

    // -- WP PINTLS -- (owner decision 2026-11-17) a hub once pinned stays TLS-only

    /// The core's unpaired record keeps a pinned hub TLS-only and drops the
    /// token; a v1 token keeps nothing.  `tls_required_pinned` offers only
    /// Forget Hub, whose words say it is the way back to no encryption.
    func testPINTLSPairAgainKeepsAPinnedHubTLSOnlyAndForgetIsTheWayBack() throws {
        let link = try secswLink()
        let kept = linkCredentialUnpaired(credential: link.credential)
        XCTAssertTrue(kept.hasPrefix("v2."), kept)
        XCTAssertTrue(linkCredentialTlsOnly(credential: kept))
        XCTAssertEqual(linkCredentialHubKey(credential: kept), "", "no key enforced until the next pairing")
        XCTAssertEqual(linkCredentialUnpaired(credential: kept), kept, "Pair Again twice: the same")
        XCTAssertEqual(linkCredentialUnpaired(credential: "a-v1-token"), "")
        XCTAssertFalse(linkCredentialTlsOnly(credential: "a-v1-token"))

        XCTAssertEqual(LinkRepair.forgetCode, "tls_required_pinned")
        XCTAssertEqual(LinkRepair.code(of: .Link(code: "tls_required_pinned", msg: "x")), "tls_required_pinned")
        XCTAssertEqual(LinkRepair.actionTitle(code: "tls_required_pinned", fromPairLink: false), "Forget Hub")
        let text = LinkRepair.text(code: "tls_required_pinned", hubName: "Studio", fromPairLink: false)
        XCTAssertTrue(text.contains("Forget Hub") && text.contains("without encryption"), text)
        XCTAssertTrue(text.contains("not even the PIN"), text)
        XCTAssertEqual(LinkRepair.code(ofCloseReason: "this hub was paired with encryption before, so it is only "
                                       + "reached encrypted, and it refused encryption"), "tls_required_pinned")
        XCTAssertTrue(LinkRepair.text(code: "tls_refused", hubName: "Studio", fromPairLink: false)
                        .contains("encrypted connection only"))
    }

    // -- WP SWPIN -- (review-pintls N2) only a record with a token is a pairing
    func testSWPINOnlyARecordWithATokenCountsAsPaired() throws {
        let kept = linkCredentialUnpaired(credential: try secswLink().credential)
        XCTAssertTrue(linkCredentialTlsOnly(credential: kept))
        XCTAssertFalse(LinkRecord.isPaired(kept), "pinned before: no token, asks for its PIN")
        XCTAssertFalse(LinkRecord.isPaired(nil))
        XCTAssertFalse(LinkRecord.isPaired(""))
        XCTAssertTrue(LinkRecord.isPaired("a-v1-token"))
    }

    // MARK: Pairing links

    /// The core parses a link; a wrong `k=` (or no key, or another scheme) is
    /// refused there, so nothing is confirmed, dialled or sent.
    func testSECSWPairingLinksGoThroughTheCoresParserAndAWrongKeyIsRefused() throws {
        let link = try secswLink(address: "192.168.1.20:11915", pin: "123456")
        XCTAssertEqual(link.hubId, "abcdef01-0000-4000-8000-00000000c0de")
        XCTAssertEqual(link.host, "192.168.1.20")
        XCTAssertEqual(link.port, 11915)
        XCTAssertEqual(link.pin, "123456")
        XCTAssertTrue(LinkPairLinks.confirmText(link).contains(link.hubKey))
        XCTAssertEqual(LinkPairLinks.address(link), "192.168.1.20:11915")

        let bad = [
            "dspi://pair?h=abc&a=127.0.0.1:11915&k=short",                       // a wrong k=
            "dspi://pair?h=abc&a=127.0.0.1:11915&k=\(secswForeignKey)A",          // 33 bytes
            "dspi://pair?h=abc&a=127.0.0.1:11915",                                // no key
            "dspi://other?h=abc&a=127.0.0.1:11915&k=\(secswForeignKey)",
        ]
        for text in bad {
            let url = try XCTUnwrap(URL(string: text))
            XCTAssertTrue(LinkPairLinks.handles(url))
            guard case .failure(let why) = LinkPairLinks.parse(url) else { return XCTFail("parsed: \(text)") }
            XCTAssertTrue(why.message.hasPrefix("Not a valid pairing link"), why.message)
        }
        XCTAssertFalse(LinkPairLinks.handles(URL(string: "https://example.com/pair")!))
    }

    /// `LinkPairLinks.open`: a wrong `k=` posts the refusal and never asks the
    /// person; a good link is confirmed first, and Cancel dials nothing.
    @MainActor
    func testSECSWOpeningAWrongLinkAsksNothingAndCancelDialsNothing() throws {
        let ui = SECSWFakeUI()
        let notices = NoticeCenter(infoSeconds: 60)
        let composite = CompositeTransport(local: FakeTransport(autoSelect: false), tokens: secswTokens(),
                                           makeRemote: { hub in
                                               XCTFail("nothing dialled")
                                               return CoreLink.transport(for: hub, tokens: secswTokens(), clientName: "SECSW")
                                           })
        let coordinator = LinkClientCoordinator(composite: composite, defaults: UserDefaults(suiteName: "secsw-\(UUID())")!)
        LinkPairLinks.open(URL(string: "dspi://pair?h=abc&a=127.0.0.1:1&k=short")!, ui: ui,
                           coordinator: coordinator, notices: notices)
        XCTAssertEqual(ui.links.count, 0, "refused before any question")
        XCTAssertEqual(notices.last?.kind, .error)
        XCTAssertTrue(notices.last?.title.hasPrefix("Not a valid pairing link") == true, "\(String(describing: notices.last))")

        ui.linkAnswer = false
        let good = "dspi://pair?h=abcdef01-0000-4000-8000-00000000c0de&a=127.0.0.1:1&k=\(secswForeignKey)"
        LinkPairLinks.open(URL(string: good)!, ui: ui, coordinator: coordinator, notices: notices)
        XCTAssertEqual(ui.links.count, 1, "confirmed first")
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        XCTAssertTrue(coordinator.manualAddresses.isEmpty, "Cancel adds and dials nothing")
    }

    // MARK: The -CoreTransport NO fallback

    /// The plaintext-only debug client refuses a hub whose record says TLS is
    /// required (its advert: a wss:// URL; a pinned v2 credential), before any
    /// connection, so no token goes out unencrypted.
    func testSECSWTheFallbackRefusesATLSRequiredRecord() throws {
        let client = LinkClientFake()
        let tokens = secswTokens()
        defer { tokens.removeToken(forHub: "hub-1") }
        let t = NetworkTransport(client: client, hubID: "hub-1", hubName: "Studio", tokens: tokens, clientName: "SECSW")
        t.connect(to: URL(string: "wss://10.0.0.2:11915/dspi/v1")!)
        XCTAssertTrue(client.connectCalls.isEmpty, "an advertised-TLS hub is not dialled")
        XCTAssertTrue(t.errorMessage?.contains("encrypted connection") == true, "\(String(describing: t.errorMessage))")

        tokens.setToken(try secswLink(hub: "hub-1").credential, forHub: "hub-1")
        t.connect(to: URL(string: "ws://10.0.0.2:11915/dspi/v1")!)
        XCTAssertTrue(client.connectCalls.isEmpty, "a pinned hub is not dialled in plaintext")

        tokens.setToken("v1-token", forHub: "hub-1")
        t.connect(to: URL(string: "ws://10.0.0.2:11915/dspi/v1")!)
        XCTAssertEqual(client.connectCalls.count, 1, "an upstream (plaintext) hub still connects")
        XCTAssertEqual(client.connectCalls.first?.1, "v1-token")
        XCTAssertNil(NetworkTransport.plaintextRefusal(url: URL(string: "ws://h:1/dspi/v1")!, credential: nil,
                                                       hubName: "h"))
    }

    // MARK: SECFWV

    /// A file that is not the bundled image gets the core's "not verified"
    /// label; the label is information, never a refusal.
    func testSECSWAFileThatIsNotBundledIsLabelledNotVerified() {
        let notice = FirmwareImage.notVerifiedNotice(Data(repeating: 0x55, count: 1024), name: "my-build.uf2")
        XCTAssertEqual(notice?.key, "firmware.not_verified")
        XCTAssertEqual(notice?.title, "Not a verified DSPi Console image")
        #if os(macOS)
        // The app's own bundled images are verified.
        for chip in BootloaderBoard.Chip.allCases {
            guard let image = try? FirmwareImage.bundled(for: chip),
                  let bytes = try? Data(contentsOf: image.url) else { continue }
            XCTAssertNil(FirmwareImage.notVerifiedNotice(bytes, name: image.url.lastPathComponent),
                         "\(image.url.lastPathComponent) is the bundled file")
        }
        #endif
    }
}

// MARK: - macOS: CoreLink against the Mac's Swift hub (plaintext, loopback)

#if os(macOS)
private final class SECSWFakeHubDevice: DSPi_Console.HubDevice {
    let fake: FakeTransport
    let info: HubDeviceInfo
    var generation: UInt64 { fake.generation }
    var isConnected: Bool { fake.isConnected }

    init(_ fake: FakeTransport) {
        self.fake = fake
        info = HubDeviceInfo(serial: FakeTransport.demoDevice.serial, platform: 1, firmware: "1.1.6", firmwareBeta: 0,
                             outputs: 9, inputs: 2, wireVersion: nil, link: .usb, boardName: "SECSW demo")
    }

    func execute(_ request: LinkCmdRequest) -> LinkCmdResponse {
        switch request.direction {
        case .get:
            switch fake.getControlResult(request: request.bRequest, value: request.wValue, index: request.wIndex,
                                         length: request.wLength) {
            case .success(let data): return LinkCmdResponse(tag: request.tag, status: .ok, payload: data)
            case .failure(let status): return LinkCmdResponse(tag: request.tag, status: status)
            }
        case .set:
            fake.sendControlRequest(request: request.bRequest, value: request.wValue, index: request.wIndex,
                                    data: request.payload)
            return LinkCmdResponse(tag: request.tag, status: .ok)
        }
    }
}

/// The Swift hub (no TLS) on loopback over the demo device.
private final class SECSWSwiftHub {
    let usb = CoreUsb(startMonitoring: false)
    let auth: LinkAuthStore
    let hub: LinkHub
    let server: LinkServer
    let fake: FakeTransport
    let port: Int

    init() throws {
        let auth = LinkAuthStore(storeURL: FileManager.default.temporaryDirectory
            .appendingPathComponent("secsw-hub-\(UUID().uuidString).json"))
        let policy = LinkPolicy.bundled ?? LinkPolicy.empty
        let hub = LinkHub(usb: usb, policy: policy, auth: auth)
        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        secswWait(5, "the demo device") { fake.isConnected }
        hub.attachDevice(SECSWFakeHubDevice(fake))
        let server = LinkServer(hub: hub, auth: auth, policy: policy)
        try server.start(port: 0)
        guard let p = server.boundPort else { throw XCTSkip("the hub did not bind a port") }
        self.auth = auth
        self.hub = hub
        self.fake = fake
        self.server = server
        port = p
    }

    deinit { server.stop() }

    var hubID: String { auth.hubID.uuidString.lowercased() }
    var url: URL { URL(string: "ws://127.0.0.1:\(port)/dspi/v1")! }
    var serial: String { FakeTransport.demoDevice.serial }
}

final class CoreLinkSECSWSwiftHubTests: XCTestCase {
    private func client(_ hub: SECSWSwiftHub, id: String? = nil, expected: String? = nil, tokens: LinkTokenStore,
                        ui: SECSWFakeUI, center: LinkSecurityCenter) -> (CoreLink, CoreBackedTransport) {
        let link = CoreLink(hubID: id ?? hub.hubID, hubName: "SECSW hub", expectedHubID: expected ?? hub.hubID,
                            tokens: tokens, clientName: "SECSW test")
        link.securityCenter = center
        link.notices = NoticeCenter(infoSeconds: 60)
        let t = CoreBackedTransport(link: .link(link))
        t.setPairingUI(ui)
        return (link, t)
    }

    /// A plaintext pairing on loopback: the PIN prompt says the connection
    /// is not encrypted (no key), the bar's state is `local` (not red), and
    /// the token is kept.
    func testSECSWAPlaintextLoopbackPairingIsLocalAndShowsNoKey() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        ui.pins = [hub.auth.beginPairing()]
        let center = LinkSecurityCenter()
        let (link, t) = client(hub, tokens: tokens, ui: ui, center: center)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "pairing and the bind") { t.isConnected }
        XCTAssertEqual(ui.pinAsks.count, 1)
        XCTAssertEqual(ui.pinAsks.first?.hubKey, "", "no TLS, no key to show")
        XCTAssertEqual(ui.pinAsks.first?.hubName, "SECSW hub")
        XCTAssertNotNil(tokens.token(forHub: hub.hubID))
        XCTAssertEqual(link.security?.encrypted, false)
        XCTAssertEqual(link.security?.local, true)
        XCTAssertEqual(center.byHub[hub.hubID], .local, "loopback: not red")
        XCTAssertTrue(ui.repairs.isEmpty)
        t.disconnect()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        XCTAssertNil(center.byHub[hub.hubID], "no session, no entry")
    }

    /// A confirmed pairing link whose key the hub does not present (this hub
    /// has no TLS at all): the dial fails with a pair-again code before any
    /// PIN prompt, and nothing is stored.
    func testSECSWAPairingLinkWithAKeyTheHubDoesNotPresentIsRefusedBeforeAnyPIN() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        let center = LinkSecurityCenter()
        let link = try secswLink(hub: hub.hubID, address: "127.0.0.1:\(hub.port)", pin: "123456")
        let (c, t) = client(hub, id: "manual:127.0.0.1:\(hub.port)", expected: "", tokens: tokens, ui: ui, center: center)
        XCTAssertNil(t.connect(pairLink: link))
        secswWait(20, "the refusal") { !ui.repairs.isEmpty }
        let code = try XCTUnwrap(ui.repairs.first?.code)
        XCTAssertTrue(LinkRepair.codes.contains(code), code)
        XCTAssertEqual(ui.pinAsks.count, 0, "no PIN prompt, no PIN sent")
        XCTAssertNil(tokens.token(forHub: hub.hubID))
        XCTAssertNil(tokens.token(forHub: "manual:127.0.0.1:\(hub.port)"))
        XCTAssertFalse(t.isConnected)
        secswWait(5, "Cancel drops the link") { c.pairLink == nil }
        XCTAssertNil(center.byHub[c.hubID])
    }

    /// A pinned (v2) credential to a hub that serves no TLS: no plaintext
    /// fallback, only pairing again; Cancel leaves it.  -- WP PINTLS -- Pair
    /// Again keeps the hub TLS-only: refused with `tls_required_pinned` before
    /// any PIN, the record still TLS-only; only Forget Hub then pairs it in
    /// plaintext (the owner decision 2026-11-17).
    func testSECSWAPinnedHubWithoutTLSOffersOnlyPairingAgainAndPairingAgainPairs() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        tokens.setToken(try secswLink(hub: hub.hubID).credential, forHub: hub.hubID)
        let ui = SECSWFakeUI()
        let center = LinkSecurityCenter()
        let (c, t) = client(hub, tokens: tokens, ui: ui, center: center)
        ui.repairAnswers = [false]
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the refusal") { ui.repairs.count == 1 }
        XCTAssertEqual(ui.pinAsks.count, 0)
        XCTAssertTrue(tokens.token(forHub: hub.hubID)?.hasPrefix("v2.") == true, "Cancel keeps the pin")
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        XCTAssertFalse(t.isConnected, "never connected in plaintext")
        secswWait(5, "Cancel answered") { c.repairCode == nil }

        // -- WP PINTLS -- Pair Again (yes), then Forget Hub declined
        ui.repairAnswers = [true, false]
        t.bind(serial: hub.serial)   // the person picks the device again
        secswWait(20, "the TLS-only refusal") { ui.repairs.count == 3 }
        XCTAssertEqual(ui.repairs[2].code, "tls_required_pinned")
        XCTAssertEqual(ui.repairs[2].actionTitle, "Forget Hub")
        XCTAssertEqual(ui.pinAsks.count, 0, "no PIN prompt, no PIN sent in plaintext")
        let kept = try XCTUnwrap(tokens.token(forHub: hub.hubID))
        XCTAssertTrue(linkCredentialTlsOnly(credential: kept), "still TLS-only: \(kept)")
        XCTAssertFalse(t.isConnected)
        secswWait(5, "Cancel answered") { c.repairCode == nil }

        // Forget Hub: the record goes, and the hub pairs unencrypted
        ui.repairAnswers = [true]
        ui.pins = [hub.auth.beginPairing()]
        t.bind(serial: hub.serial)
        secswWait(20, "the refusal again") { ui.repairs.count == 4 }
        XCTAssertEqual(ui.repairs[3].code, "tls_required_pinned")
        secswWait(20, "pairing after Forget Hub and the bind") { t.isConnected }
        XCTAssertEqual(ui.pinAsks.count, 1, "the PIN after Forget Hub")
        let stored = try XCTUnwrap(tokens.token(forHub: hub.hubID))
        XCTAssertFalse(stored.hasPrefix("v2."), "the hub's new (plaintext) pairing")
        t.disconnect()
    }

    // -- WP SWPIN -- (review-pintls F4) a typed address with no expected id

    /// A typed address whose `/info` did not load (no expected id), at a hub
    /// whose record (under the id its hello names) is the pinned-before one:
    /// dialled again with the record, refused `tls_required_pinned` (this hub
    /// has no TLS) before any PIN; no plaintext session, no PIN prompt.
    func testSWPINATypedAddressIsHeldToTheRecordOfTheHubItsHelloNames() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let kept = linkCredentialUnpaired(credential: try secswLink(hub: hub.hubID).credential)
        tokens.setToken(kept, forHub: hub.hubID)
        let ui = SECSWFakeUI()
        ui.repairAnswers = [false]
        let center = LinkSecurityCenter()
        let manual = "manual:127.0.0.1:\(hub.port)"
        let (c, t) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the TLS-only refusal") { !ui.repairs.isEmpty }
        XCTAssertEqual(ui.repairs.first?.code, "tls_required_pinned")
        XCTAssertEqual(ui.pinAsks.count, 0, "no PIN prompt on a plaintext session to a hub pinned before")
        XCTAssertEqual(c.expectedHubID, hub.hubID, "the hello's id became the expected one")
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        XCTAssertFalse(t.isConnected, "never connected in plaintext")
        XCTAssertNil(center.byHub[manual], "no session was applied")
        XCTAssertEqual(tokens.token(forHub: hub.hubID), kept, "the record is kept")
        XCTAssertNil(tokens.token(forHub: manual))
    }

    /// The same typed address with a (plaintext) token stored under the hub's
    /// id only: the hub asks for a PIN on the first dial, which is dialled
    /// again with the token, so it connects without asking.
    func testSWPINATypedAddressUsesTheTokenStoredUnderTheHubsID() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        ui.pins = [hub.auth.beginPairing()]
        let center = LinkSecurityCenter()
        let (_, first) = client(hub, tokens: tokens, ui: ui, center: center)
        first.connect(to: hub.url)
        first.bind(serial: hub.serial)
        secswWait(20, "the first pairing") { first.isConnected }
        first.disconnect()
        RunLoop.main.run(until: Date().addingTimeInterval(0.3))
        XCTAssertEqual(ui.pinAsks.count, 1)
        let token = try XCTUnwrap(tokens.token(forHub: hub.hubID))
        XCTAssertFalse(linkCredentialTlsOnly(credential: token))

        let manual = "manual:127.0.0.1:\(hub.port)"
        let (c, t) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the session with the stored token") { t.isConnected }
        XCTAssertEqual(ui.pinAsks.count, 1, "no second PIN prompt")
        XCTAssertEqual(c.expectedHubID, hub.hubID)
        XCTAssertTrue(ui.repairs.isEmpty)
        t.disconnect()
    }

    // -- review fu1 -- (F1) the copy under a typed address's key

    /// A hub first paired at a typed address without TLS keeps a copy of its
    /// token under the address's key.  Pinned later through another entry
    /// (only the record under its own id follows), the copy is stale: with no
    /// expected id it must not go out unencrypted to whoever answers there.
    /// The hub sees no token; the record then refuses the plaintext session.
    func testREVIEWFU1AStaleTokenUnderATypedAddressIsNotSentToAHubSincePinned() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        ui.pins = [hub.auth.beginPairing()]
        ui.repairAnswers = [false]
        let center = LinkSecurityCenter()
        let manual = "manual:127.0.0.1:\(hub.port)"
        let (_, first) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        first.connect(to: hub.url)
        first.bind(serial: hub.serial)
        secswWait(20, "the first pairing") { first.isConnected }
        first.disconnect()
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        let stale = try XCTUnwrap(tokens.token(forHub: manual))
        XCTAssertFalse(linkCredentialTlsOnly(credential: stale))
        let kept = linkCredentialUnpaired(credential: try secswLink(hub: hub.hubID).credential)
        tokens.setToken(kept, forHub: hub.hubID)
        let before = hub.auth.clients.map { $0.lastSeen }
        RunLoop.main.run(until: Date().addingTimeInterval(1.2))   // a later use would stamp another second
        let (c, t) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the TLS-only refusal") { !ui.repairs.isEmpty }
        XCTAssertEqual(ui.repairs.first?.code, "tls_required_pinned")
        XCTAssertEqual(c.expectedHubID, hub.hubID)
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        XCTAssertFalse(t.isConnected)
        XCTAssertEqual(ui.pinAsks.count, 1, "only the first pairing asked")
        XCTAssertEqual(hub.auth.clients.map { $0.lastSeen }, before, "the hub was shown no token")
    }

    /// A token kept only under the typed address's key (no record under the
    /// hub's id) still connects without a PIN: held back on the first dial,
    /// sent on the second, once the hub has named itself.
    func testREVIEWFU1ATokenOnlyUnderATypedAddressStillConnectsWithoutAPIN() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        ui.pins = [hub.auth.beginPairing()]
        let center = LinkSecurityCenter()
        let manual = "manual:127.0.0.1:\(hub.port)"
        let (_, first) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        first.connect(to: hub.url)
        first.bind(serial: hub.serial)
        secswWait(20, "the first pairing") { first.isConnected }
        first.disconnect()
        RunLoop.main.run(until: Date().addingTimeInterval(0.5))
        tokens.removeToken(forHub: hub.hubID)
        XCTAssertNotNil(tokens.token(forHub: manual))
        let (c, t) = client(hub, id: manual, expected: "", tokens: tokens, ui: ui, center: center)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the session with the address's token") { t.isConnected }
        XCTAssertEqual(ui.pinAsks.count, 1, "no second PIN prompt")
        XCTAssertEqual(c.expectedHubID, hub.hubID)
        XCTAssertTrue(ui.repairs.isEmpty)
        t.disconnect()
    }

    /// A hub whose advert says TLS (`tls=1`) but that refuses it: `tls_refused`,
    /// no plaintext retry, no PIN.
    func testSECSWAnAdvertisedTLSHubThatRefusesTLSIsNotDialledInPlaintext() throws {
        let hub = try SECSWSwiftHub()
        let tokens = secswTokens()
        let ui = SECSWFakeUI()
        let center = LinkSecurityCenter()
        let record = DiscoveredHub(id: hub.hubID, name: "SECSW hub", kind: "console", authMode: "pin",
                                   deviceCount: 1, serials: [hub.serial], path: "/dspi/v1", tls: true,
                                   host: "127.0.0.1", port: hub.port, isManual: false)
        let t = CoreLink.transport(for: record, tokens: tokens, clientName: "SECSW test")
        XCTAssertEqual(t.linkSession?.tlsAdvertised, true)
        t.linkSession?.securityCenter = center
        t.setPairingUI(ui)
        t.connect(to: hub.url)
        t.bind(serial: hub.serial)
        secswWait(20, "the refusal") { !ui.repairs.isEmpty }
        let code = try XCTUnwrap(ui.repairs.first?.code)
        print("SECSW: an advertised-TLS hub without TLS: \(code)")
        XCTAssertTrue(LinkRepair.codes.contains(code), code)
        XCTAssertEqual(ui.pinAsks.count, 0)
        XCTAssertFalse(t.isConnected)
    }
}
#endif
