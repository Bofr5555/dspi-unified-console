import XCTest
@testable import DSPi_Console

/// WP SWF3 (hw-mac F8): the iOS app said "1.1.6 beta 5" while the Mac app and
/// the firmware said 1.1.6, so every iPhone showed a false "newer firmware"
/// banner.  Both apps carry one version, and the banner stays quiet when the
/// device runs exactly that version.
final class AppVersionTests: XCTestCase {
    /// MARKETING_VERSION of every build configuration of the two app targets,
    /// keyed by bundle identifier, read from the project file.
    private func appMarketingVersions() throws -> [String: Set<String>] {
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
        var versions: [String: Set<String>] = [:]
        for block in text.components(separatedBy: "isa = XCBuildConfiguration;").dropFirst() {
            let settings = block.prefix { $0 != "}" }
            guard let id = setting("PRODUCT_BUNDLE_IDENTIFIER", in: settings),
                  id == "WeebLabs.DSPi-Console" || id == "WeebLabs.DSPi-Console-iOS" else { continue }
            versions[id, default: []].insert(setting("MARKETING_VERSION", in: settings) ?? "missing")
        }
        return versions
    }

    func testBothAppsCarryOneVersion() throws {
        let versions = try appMarketingVersions()
        XCTAssertEqual(Set(versions.keys), ["WeebLabs.DSPi-Console", "WeebLabs.DSPi-Console-iOS"])
        let all = versions.values.reduce(into: Set<String>()) { $0.formUnion($1) }
        XCTAssertEqual(all.count, 1, "MARKETING_VERSION differs between the apps or configurations: \(versions)")
    }

    func testThisAppExpectsTheProjectVersion() throws {
        let version = try XCTUnwrap(try appMarketingVersions().values.first?.first)
        XCTAssertEqual(FirmwareVersion.expected, FirmwareVersion(version))
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
