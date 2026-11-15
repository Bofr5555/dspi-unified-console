import CryptoKit
import XCTest
@testable import DSPi_Console

/// WP SECREL: the updater trusts the fork's key, not Weeb Labs', and its feed
/// override must be https or loopback (PLAN-SEC N12, OD-SECj).
///
/// Until the owner makes the fork's update key (OD-SECj) the app embeds a TEST
/// key. Its private half is derived here from a fixed text, so it is public
/// knowledge and a build carrying it must never install anything.
final class AppUpdateSECRELTests: XCTestCase {
    /// Weeb Labs' upstream public key, which the updater embedded before SECREL.
    /// Their release v1.1.6 (DSPi.Console.v1.1.6.zip.sig) verifies with it.
    static let upstreamPublicKey = Data(base64Encoded: "1PVzcgAEI2T4cMy+DWeLEQaxdXRywAg+s+MbYNlffWU=")!

    /// TEST KEY: the seed is SHA-256 of this text. Never for a release.
    static let testSeedText = "uConsole for DSPi update TEST KEY (WP SECREL), never for a release"

    static func testPrivateKey() throws -> Curve25519.Signing.PrivateKey {
        try Curve25519.Signing.PrivateKey(rawRepresentation: Data(SHA256.hash(data: Data(testSeedText.utf8))))
    }

    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("AppUpdateSECRELTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    /// A real zip, made the way release zips are (ditto -c -k --keepParent).
    private func makeZip() throws -> Data {
        let app = dir.appendingPathComponent("uConsole for DSPi.app/Contents")
        try FileManager.default.createDirectory(at: app, withIntermediateDirectories: true)
        try Data("release payload".utf8).write(to: app.appendingPathComponent("payload"))
        let zip = dir.appendingPathComponent("release.zip")
        XCTAssertEqual(AppUpdateInstaller.run("/usr/bin/ditto",
                                              ["-c", "-k", "--keepParent", dir.appendingPathComponent("uConsole for DSPi.app").path, zip.path]), 0)
        return try Data(contentsOf: zip)
    }

    // MARK: Keys

    func testSECRELTestKeyConstantMatchesItsDocumentedSeed() throws {
        XCTAssertEqual(try Self.testPrivateKey().publicKey.rawRepresentation, AppUpdateInstaller.testPublicKey)
    }

    func testSECRELEmbeddedKeyIsNotUpstreams() {
        XCTAssertNotEqual(AppUpdateInstaller.publicKey, Self.upstreamPublicKey)
        XCTAssertNotEqual(AppUpdateInstaller.testPublicKey, Self.upstreamPublicKey)
        XCTAssertNotNil(try? Curve25519.Signing.PublicKey(rawRepresentation: AppUpdateInstaller.publicKey))
    }

    /// Done criterion: a zip signed with the TEST key verifies with the
    /// embedded key. Once the owner's key is swapped in this checks the TEST
    /// key no longer verifies instead.
    func testSECRELZipSignedWithTheTestKeyVerifies() throws {
        let zip = try makeZip()
        let signature = try Self.testPrivateKey().signature(for: zip).base64EncodedString() + "\n"
        if AppUpdateInstaller.usesTestKey {
            XCTAssertTrue(AppUpdateInstaller.signatureIsValid(zip, signatureText: signature))
            XCTAssertFalse(AppUpdateInstaller.signatureIsValid(zip + Data([0]), signatureText: signature))
        } else {
            XCTAssertFalse(AppUpdateInstaller.signatureIsValid(zip, signatureText: signature),
                           "the release key must not accept the public TEST key")
        }
    }

    /// Done criterion: a zip signed with upstream's key does not verify.
    /// Upstream's private key is not ours, so no upstream signature can be
    /// made here: a stand-in key plays upstream (valid under its own public
    /// half, refused by the embedded key), and the test above shows the
    /// embedded key is not upstream's. The real upstream v1.1.6 zip and .sig
    /// were checked with openssl: valid under upstream's key, refused under
    /// the TEST key (WP SECREL report).
    func testSECRELZipSignedWithAnotherKeyDoesNotVerify() throws {
        let zip = try makeZip()
        let other = Curve25519.Signing.PrivateKey()
        let signature = try other.signature(for: zip).base64EncodedString()
        XCTAssertTrue(AppUpdateInstaller.signatureIsValid(zip, signatureText: signature,
                                                          publicKey: other.publicKey.rawRepresentation))
        XCTAssertFalse(AppUpdateInstaller.signatureIsValid(zip, signatureText: signature))
        // And the reverse: what the embedded TEST key accepts, upstream's key refuses.
        let testSignature = try Self.testPrivateKey().signature(for: zip).base64EncodedString()
        XCTAssertFalse(AppUpdateInstaller.signatureIsValid(zip, signatureText: testSignature,
                                                           publicKey: Self.upstreamPublicKey))
    }

    @MainActor
    func testSECRELTestKeyBuildNeverInstalls() {
        XCTAssertEqual(AppUpdateInstaller.usesTestKey, AppUpdateInstaller.publicKey == AppUpdateInstaller.testPublicKey)
        let url = URL(string: "https://example.invalid/r")!
        let release = AppUpdateChecker.Release(tagName: "v9.9.9", htmlURL: url, draft: false,
                                               assets: [.init(name: "a.zip", url: url), .init(name: "a.zip.sig", url: url)])
        XCTAssertNotNil(AppUpdateInstaller.signedZip(in: release))
        if AppUpdateInstaller.usesTestKey {
            XCTAssertFalse(AppUpdateChecker.shared.canInstall(release))
            XCTAssertNotNil(AppUpdateInstaller.Failure.testKeyBuild.errorDescription)
        }
    }

    // MARK: Feed override

    func testSECRELHttpOverrideIsRefused() {
        for text in ["http://example.com/releases", "http://192.168.1.10:8000/releases", "http://10.0.0.1/r",
                     "http://127.example.com/r", "http://128.0.0.1/r", "http://127.0.0/r", "http://localhost.evil.com/r",
                     "file:///tmp/releases.json", "ftp://localhost/r", "https:///nohost", "not a url", ""] {
            XCTAssertNil(AppUpdateChecker.acceptedFeedOverride(text), text)
        }
        XCTAssertNil(AppUpdateChecker.acceptedFeedOverride(nil))
    }

    func testSECRELHttpsAndLoopbackOverridesAreAccepted() {
        for text in ["https://example.com/releases", "HTTPS://Example.com/r", "http://localhost:8000/releases",
                     "http://127.0.0.1:8000/releases", "http://127.1.2.3/r", "http://[::1]:8000/releases"] {
            XCTAssertEqual(AppUpdateChecker.acceptedFeedOverride(text)?.absoluteString, text, text)
        }
    }
}
