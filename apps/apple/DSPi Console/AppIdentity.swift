//
//  AppIdentity.swift
//  DSPi Console
//
//  WP REL013M: the names this app files its logs, data and keychain items
//  under.  The macOS app has the unified console's own bundle identifier
//  (invalid.dspi-unified-console.DSPiConsole, as the Qt app), so it keeps
//  nothing in Weeb Labs' DSPi Console's places and both can be installed on
//  one Mac.  The iOS app keeps its earlier names.
//

import Foundation

enum AppIdentity {
    /// The running app's bundle identifier (under XCTest, the host app's).
    static let bundleID = Bundle.main.bundleIdentifier ?? "invalid.dspi-unified-console.DSPiConsole"

    /// The unified logging subsystem of every `Logger` in the app.
    static let logSubsystem = bundleID

    /// The folder in Application Support that holds the app's files.  The
    /// macOS app is not sandboxed, so that folder is shared by every app of
    /// the user: it is named after the bundle identifier there.
    static func supportFolder(legacy: String) -> String {
        #if os(macOS)
        return bundleID
        #else
        return legacy
        #endif
    }
}
