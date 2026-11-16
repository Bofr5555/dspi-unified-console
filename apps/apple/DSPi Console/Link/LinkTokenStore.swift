//
//  LinkTokenStore.swift
//  DSPi Console
//
//  Keeps the pairing credential for each hub in the keychain, keyed by the
//  hub's id rather than its address, so a hub that moves to a new IP is still
//  recognised.  See spec 6.2.
//
//  -- WP SECSW -- What is stored is the core's credential (PLAN-SEC 4.3): a
//  bare token (`v1`) or a `v2.<token>.<pin>...` string carrying the hub key's
//  pin, as `link_pair`, the token listener and `link_security().credential`
//  hand it over.  The item is `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`
//  and not synchronizable: never in iCloud Keychain, never restored to another
//  device from a backup.  An item written before this class is moved to it the
//  next time it is read or written.
//
//  macOS: the accessibility class lives only in the data-protection keychain,
//  which needs an application identifier (a Team ID's signature).  A build
//  without one (ad-hoc signed: every build today) gets
//  `errSecMissingEntitlement` there and keeps using the login keychain, which
//  has no such class and never syncs; `backend` says which one is in use.
//

import Foundation
import Security

final class LinkTokenStore {
    private let service: String

    /// -- WP SECSW -- The protection class of every item.
    static let accessibility = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String

    /// -- WP SECSW -- Where the items live.
    enum Backend: Equatable {
        /// iOS, or a macOS build signed with an application identifier: the
        /// items carry `accessibility`.
        case dataProtection
        /// macOS without one: the login keychain (no protection class).
        case loginKeychain
    }

    /// -- WP REL013M -- On macOS the items are named after this app's bundle
    /// identifier, apart from Weeb Labs' DSPi Console's in the same keychain.
    static var defaultService: String {
        #if os(macOS)
        return AppIdentity.bundleID + ".link"
        #else
        return "com.weeblabs.dspi-console.link"
        #endif
    }

    /// `service` namespaces the keychain items; tests pass their own so they
    /// never touch the real tokens.
    init(service: String = LinkTokenStore.defaultService) {
        self.service = service
    }

    /// -- WP SECSW -- The keychain in use (probed once per process on macOS).
    var backend: Backend { Self.resolvedBackend }

    private static let resolvedBackend: Backend = {
        #if os(macOS)
        // A throwaway item tells whether this build may use the
        // data-protection keychain.
        let probe: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: defaultService + ".probe",
            kSecAttrAccount as String: UUID().uuidString,
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrAccessible as String: accessibility,
            kSecValueData as String: Data([0]),
        ]
        guard SecItemAdd(probe as CFDictionary, nil) == errSecSuccess else { return .loginKeychain }
        var gone = probe
        gone[kSecValueData as String] = nil
        SecItemDelete(gone as CFDictionary)
        return .dataProtection
        #else
        return .dataProtection
        #endif
    }()

    // -- WP SECSWF -- (review N5) The credentials this process read or wrote,
    // by service and hub, a miss included: the hub lists' badges and every
    // status look ask on main, and only the first ask reaches the Keychain.
    // Every instance with the same service shares it; writes go through.
    private static var cache: [String: String?] = [:]
    private static let cacheLock = NSLock()
    private func cacheKey(_ hubID: String) -> String { service + "\u{0}" + hubID }
    private func cached(_ hubID: String) -> String?? {
        Self.cacheLock.lock(); defer { Self.cacheLock.unlock() }
        return Self.cache[cacheKey(hubID)]
    }
    private func remember(_ token: String?, forHub hubID: String) {
        Self.cacheLock.lock(); defer { Self.cacheLock.unlock() }
        Self.cache[cacheKey(hubID)] = .some(token)
    }

    /// The stored credential (a token or a `v2` string), nil when none.
    func token(forHub hubID: String) -> String? {
        if let known = cached(hubID) { return known }   // -- WP SECSWF --
        let token = readToken(forHub: hubID)
        remember(token, forHub: hubID)
        return token
    }

    private func readToken(forHub hubID: String) -> String? {
        var query = baseQuery(hubID)
        query[kSecReturnData as String] = true
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
              let item = out as? [String: Any],
              let data = item[kSecValueData as String] as? Data else { return nil }
        // -- WP SECSW -- an older item is moved to the protection class
        if backend == .dataProtection,
           item[kSecAttrAccessible as String] as? String != Self.accessibility {
            SecItemUpdate(baseQuery(hubID) as CFDictionary, protection as CFDictionary)
        }
        return String(data: data, encoding: .utf8)
    }

    /// Store `token` (the core's credential) for the hub.
    func setToken(_ token: String, forHub hubID: String) {
        let data = Data(token.utf8)
        var query = baseQuery(hubID)
        var update = protection
        update[kSecValueData as String] = data
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            query.merge(protection) { _, new in new }
            query[kSecValueData as String] = data
            SecItemAdd(query as CFDictionary, nil)
        }
        remember(token, forHub: hubID)   // -- WP SECSWF --
    }

    func removeToken(forHub hubID: String) {
        SecItemDelete(baseQuery(hubID) as CFDictionary)
        remember(nil, forHub: hubID)   // -- WP SECSWF --
    }

    /// -- WP SECSW -- The item's attributes as the keychain reports them (the
    /// tests assert them), nil when there is none.
    func attributes(forHub hubID: String) -> [String: Any]? {
        var query = baseQuery(hubID)
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess else { return nil }
        return out as? [String: Any]
    }

    /// -- WP SECSW -- The attributes every write sets: the protection class
    /// where the keychain has one, and never synchronizable.
    private var protection: [String: Any] {
        var p: [String: Any] = [kSecAttrSynchronizable as String: false]
        if backend == .dataProtection { p[kSecAttrAccessible as String] = Self.accessibility }
        return p
    }

    private func baseQuery(_ hubID: String) -> [String: Any] {
        var q: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                kSecAttrService as String: service,
                                kSecAttrAccount as String: hubID.lowercased()]
        #if os(macOS)
        if backend == .dataProtection { q[kSecUseDataProtectionKeychain as String] = true }
        #endif
        return q
    }
}
