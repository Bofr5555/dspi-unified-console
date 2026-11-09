import XCTest
@testable import DSPi_Console

/// Which GitHub release the update check offers.  Betas are told apart by tag
/// alone, since the releases are not flagged as pre-releases.
final class AppUpdateCheckerTests: XCTestCase {
    /// A release; with `macApp` it carries the Swift app's .dmg (WP REL013M).
    private func release(_ tag: String, draft: Bool = false, macApp: Bool = true) -> AppUpdateChecker.Release {
        let base = "https://github.com/Bofr5555/dspi-unified-console/releases"
        let version = tag.hasPrefix("v") ? String(tag.dropFirst()) : tag
        let dmg = "DSPi-uConsole-Swift-\(version)-macos.dmg"
        return AppUpdateChecker.Release(tagName: tag,
                                        htmlURL: URL(string: "\(base)/tag/\(tag)")!,
                                        draft: draft,
                                        assets: macApp ? [.init(name: dmg, url: URL(string: "\(base)/download/\(tag)/\(dmg)")!)] : [])
    }

    private func offered(_ tags: [String], over current: String) -> String? {
        AppUpdateChecker.newestUpdate(in: tags.map { release($0) },
                                      over: FirmwareVersion(current)!)?.tagName
    }

    func testFinalUserIsNotOfferedBetas() {
        XCTAssertNil(offered(["v1.1.6-beta4", "v1.1.5"], over: "1.1.5"))
        XCTAssertEqual(offered(["v1.1.7-beta1", "v1.1.6", "v1.1.5"], over: "1.1.5"), "v1.1.6")
    }

    func testBetaUserIsOfferedNewerBetasAndTheFinal() {
        XCTAssertEqual(offered(["v1.1.6-beta4", "v1.1.6-beta3"], over: "1.1.6-beta3"), "v1.1.6-beta4")
        XCTAssertEqual(offered(["v1.1.6", "v1.1.6-beta4"], over: "1.1.6-beta4"), "v1.1.6")
    }

    func testNothingNewerOffersNothing() {
        XCTAssertNil(offered(["v1.1.6-beta4", "v1.1.6-beta3", "v1.1.5"], over: "1.1.6-beta4"))
        XCTAssertNil(offered([], over: "1.1.6"))
    }

    func testDoubleDigitBetaSortsAboveSingleDigit() {
        XCTAssertEqual(offered(["v1.1.6-beta9", "v1.1.6-beta10"], over: "1.1.6-beta2"), "v1.1.6-beta10")
    }

    func testDraftsAndUnparseableTagsAreIgnored() {
        let releases = [release("v1.1.7", draft: true), release("nightly"), release("v1.1.6")]
        XCTAssertEqual(AppUpdateChecker.newestUpdate(in: releases, over: FirmwareVersion(1, 1, 5))?.tagName, "v1.1.6")
    }

    // -- WP REL013M -- the unified console's own tags
    func testUnifiedConsoleTagsAreCompared() {
        XCTAssertEqual(offered(["v0.14.0", "v0.13.0", "v0.10.1"], over: "0.13.0"), "v0.14.0")
        XCTAssertNil(offered(["v0.13.0", "v0.10.1"], over: "0.13.0"))
    }

    /// Weeb Labs' tag in this repository, the Android app's and any release
    /// without the Swift .dmg are no update, though "1.1.6" sorts above 0.13.0.
    func testReleasesWithoutTheMacAppAreIgnored() {
        let releases = [release("v1.1.6-beta4", macApp: false), release("android-v0.14.0", macApp: false),
                        release("v0.14.0", macApp: false), release("v0.13.1")]
        XCTAssertEqual(AppUpdateChecker.newestUpdate(in: releases, over: FirmwareVersion(0, 13, 0))?.tagName, "v0.13.1")
        XCTAssertNil(AppUpdateChecker.newestUpdate(in: Array(releases.prefix(3)), over: FirmwareVersion(0, 13, 0)))
    }

    func testTheFeedIsTheUnifiedConsolesAndOff() {
        XCTAssertEqual(AppUpdateChecker.releasesURL.path, "/repos/Bofr5555/dspi-unified-console/releases")
        XCTAssertFalse(AppUpdateChecker.enabled, "the Qt .dmg of this release does not check either")
    }

    func testDisabledCheckerAsksNothing() async throws {
        let checker = await AppUpdateChecker.shared
        let offer = try await checker.check()
        XCTAssertNil(offer)
        let version = await checker.currentVersion
        XCTAssertNotNil(version)
        XCTAssertNotEqual(version, FirmwareVersion.expected, "the app's own version, not the firmware's")
    }

    func testDecodesGitHubReleaseJSON() throws {
        let json = #"[{"tag_name":"v1.1.6-beta4","html_url":"https://github.com/WeebLabs/DSPi-Console/releases/tag/v1.1.6-beta4","draft":false,"prerelease":false,"assets":[{"name":"DSPi.Console.v1.1.6-beta4.zip","size":1,"browser_download_url":"https://github.com/WeebLabs/DSPi-Console/releases/download/v1.1.6-beta4/DSPi.Console.v1.1.6-beta4.zip"}]}]"#
        let decoded = try JSONDecoder().decode([AppUpdateChecker.Release].self, from: Data(json.utf8))
        XCTAssertEqual(decoded.first?.version, FirmwareVersion(1, 1, 6, 4))
        XCTAssertEqual(decoded.first?.assets.first?.name, "DSPi.Console.v1.1.6-beta4.zip")
        XCTAssertEqual(decoded.first?.assets.first?.url.lastPathComponent, "DSPi.Console.v1.1.6-beta4.zip")
    }
}
