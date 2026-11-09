import XCTest
@testable import DSPi_Console

/// WP SWF3 (hw-mac F8): the iOS app said "1.1.6 beta 5" while the Mac app and
/// the firmware said 1.1.6, so every iPhone showed a false "newer firmware"
/// banner.  Both apps carry one version, and the banner stays quiet when the
/// device runs exactly that version.
final class AppVersionTests: XCTestCase {
    /// The setting `key` of every build configuration of the two app targets,
    /// keyed by bundle identifier, read from the project file.
    private func appSettings(_ key: String) throws -> [String: Set<String>] {
        let project = URL(fileURLWithPath: #filePath)        // .../DSPi ConsoleTests/AppVersionTests.swift
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("DSPi Console.xcodeproj/project.pbxproj")
        let text = try String(contentsOf: project, encoding: .utf8)
        func setting(_ key: String, in block: Substring) -> String? {
            guard let line = block.split(separator: "\n").first(where: {
                $0.trimmingCharacters(in: .whitespaces).hasPrefix("\(key) = ")
            }) else { return nil }
            return line.split(separator: "=", maxSplits: 1)[1]
                .trimmingCharacters(in: CharacterSet(charactersIn: " \t;\""))
        }
        var values: [String: Set<String>] = [:]
        for block in text.components(separatedBy: "isa = XCBuildConfiguration;").dropFirst() {
            let settings = block.prefix { $0 != "}" }
            guard let id = setting("PRODUCT_BUNDLE_IDENTIFIER", in: settings),
                  id == Self.macID || id == Self.iosID else { continue }
            values[id, default: []].insert(setting(key, in: settings) ?? "missing")
        }
        return values
    }

    // -- WP REL013M -- the macOS app has the unified console's identifier and
    // version; the firmware it expects is DSPI_FIRMWARE_VERSION.  The iOS app
    // still expects its MARKETING_VERSION.
    private static let macID = "invalid.dspi-unified-console.DSPiConsole"
    private static let iosID = "WeebLabs.DSPi-Console-iOS"

    /// The firmware version each app expects, from the project file.
    private func expectedFirmware() throws -> [String: Set<String>] {
        var versions = try appSettings("MARKETING_VERSION")
        versions[Self.macID] = try appSettings("DSPI_FIRMWARE_VERSION")[Self.macID]
        return versions
    }

    func testBothAppsCarryOneVersion() throws {
        let versions = try expectedFirmware()
        XCTAssertEqual(Set(versions.keys), [Self.macID, Self.iosID])
        let all = versions.values.reduce(into: Set<String>()) { $0.formUnion($1) }
        XCTAssertEqual(all.count, 1, "The expected firmware differs between the apps or configurations: \(versions)")
    }

    func testThisAppExpectsTheProjectVersion() throws {
        let version = try XCTUnwrap(try expectedFirmware().values.first?.first)
        XCTAssertEqual(FirmwareVersion.expected, FirmwareVersion(version))
    }

    /// The macOS app's version is the release's (the workspace's Cargo.toml),
    /// its build number the minor version, as the 0.13.0 release has it.
    func testMacAppCarriesTheReleaseVersion() throws {
        let cargo = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Cargo.toml")
        let toml = try String(contentsOf: cargo, encoding: .utf8)
        let workspace = try XCTUnwrap(toml.components(separatedBy: "\n[workspace.package]").dropFirst().first)
        let line = try XCTUnwrap(workspace.split(separator: "\n").first { $0.hasPrefix("version = ") })
        let release = line.split(separator: "\"")[1]
        XCTAssertEqual(try appSettings("MARKETING_VERSION")[Self.macID], [String(release)])
        let minor = String(release.split(separator: ".")[1])
        XCTAssertEqual(try appSettings("CURRENT_PROJECT_VERSION")[Self.macID], [minor])
    }

    func testBannerOnlyForADifferentFirmware() throws {
        let expected = try XCTUnwrap(FirmwareVersion.expected)
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        vm.isDeviceConnected = true
        func match(_ v: FirmwareVersion) -> FirmwareMatch? {
            vm.firmwareVersion = (v.major, v.minor, v.patch, v.beta)
            return vm.firmwareMatch
        }
        XCTAssertEqual(match(expected), .match)
        XCTAssertEqual(match(FirmwareVersion(expected.major, expected.minor, expected.patch + 1)), .deviceNewer)
        XCTAssertEqual(match(FirmwareVersion(expected.major, expected.minor, max(0, expected.patch - 1))),
                       expected.patch > 0 ? .deviceOlder : .match)
        // The hw-mac case: a final device against a beta build of the same
        // patch is newer, so a beta version string here would bring it back.
        XCTAssertGreaterThan(FirmwareVersion(1, 1, 6), FirmwareVersion("1.1.6-beta5")!)
    }

    func testNoBannerForTheDemoDevice() {
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        vm.isDeviceConnected = true
        vm.selectedDevice = FakeTransport.demoDevice
        vm.firmwareVersion = (1, 1, 6, 5)   // the demo's capture
        XCTAssertNil(vm.firmwareMatch)
    }
}
