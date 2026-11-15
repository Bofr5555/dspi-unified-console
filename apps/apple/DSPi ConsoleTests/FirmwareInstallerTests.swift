import XCTest
import Combine
import dspi_core
@testable import DSPi_Console

/// Pure-logic tests for firmware versioning and the installer's decisions.
///
/// The installer is the one place in the app that can brick a board, and it
/// runs against hardware nobody can attach in CI, so everything it decides is
/// pushed behind a seam and tested here: which board it will act on, which it
/// refuses, whether a mid-write error is a reboot or a failure, and whether a
/// flash counts as successful. No device.
final class FirmwareInstallerTests: XCTestCase {

    // MARK: - Version parsing and ordering

    func testParsesPlainPointRelease() {
        XCTAssertEqual(FirmwareVersion("1.1.7"), FirmwareVersion(1, 1, 7))
    }

    /// The tag spelling carries the beta ordinal, which the device reports too;
    /// any other suffix still reads as a final release.
    func testParsesBetaSuffixAndIgnoresOthers() {
        XCTAssertEqual(FirmwareVersion("1.1.6-beta2"), FirmwareVersion(1, 1, 6, 2))
        XCTAssertEqual(FirmwareVersion("1.1.6-rc1"), FirmwareVersion(1, 1, 6))
    }

    func testMissingPatchIsZero() {
        XCTAssertEqual(FirmwareVersion("1.2"), FirmwareVersion(1, 2, 0))
    }

    func testRejectsNonNumeric() {
        XCTAssertNil(FirmwareVersion("unreleased"))
    }

    /// The whole reason the wire encoding was widened: patch runs past 9, and
    /// string ordering would get this backwards.
    func testOrdersPatchNumerically() {
        XCTAssertTrue(FirmwareVersion(1, 1, 9) < FirmwareVersion(1, 1, 10))
        XCTAssertTrue(FirmwareVersion(1, 1, 15) < FirmwareVersion(1, 2, 0))
    }

    // MARK: - Asset names

    func testReadsVersionFromAssetName() {
        XCTAssertEqual(FirmwareImage.versionFromAssetName("DSPi-RP2350-v1.1.7.uf2"),
                       FirmwareVersion(1, 1, 7))
        XCTAssertEqual(FirmwareImage.versionFromAssetName("DSPi-RP2040-v1.1.16.uf2"),
                       FirmwareVersion(1, 1, 16))
    }

    func testRejectsAssetNameWithoutVersion() {
        XCTAssertNil(FirmwareImage.versionFromAssetName("DSPi.uf2"))
    }

    // -- WP SW-M2 -- Reboot versus failure (a write error once the drive
    // went away is the board rebooting) is the core's now (`flash_to_drive`,
    // C11), with its tests there; Swift writes no byte.

    // MARK: - Which board it will act on

    func testNoBoardWaits() {
        let installer = makeInstaller(boards: [])
        installer.beginWatching()
        XCTAssertEqual(installer.state, .waitingForBoard)
    }

    func testOneMountedBoardIsReady() {
        let board = BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        let installer = makeInstaller(boards: [board])
        installer.beginWatching()
        XCTAssertEqual(installer.state, .ready(board))
    }

    /// The drive always mounts a moment after the board enumerates, so a
    /// missing volume starts as waiting, not as an error.  Calling it a
    /// failure immediately rejected any board plugged in while the window was
    /// already open, a beat before its drive appeared.
    func testBoardWithoutVolumeWaitsRatherThanFailing() {
        let installer = makeInstaller(boards: [BootloaderBoard(chip: .rp2040, volumeURL: nil)])
        installer.beginWatching()
        XCTAssertEqual(installer.state, .waitingForVolume(.rp2040))
    }

    /// A drive that never turns up is the likely shape of a denied
    /// removable-volume prompt, and the user can act on that, so it does
    /// eventually become an error naming the drive.
    func testBoardWithoutVolumeFailsOnceTheWaitElapses() {
        let clock = TestClock()
        let locator = FakeBootloaderLocator(boards: [BootloaderBoard(chip: .rp2040, volumeURL: nil)])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { _ in throw FirmwareInstallError.noBoardFound },
                                          now: { clock.now })
        installer.beginWatching()
        XCTAssertEqual(installer.state, .waitingForVolume(.rp2040))

        clock.advance(FirmwareInstaller.volumeWaitTimeout)
        locator.emit()
        XCTAssertEqual(installer.state, .failed(.volumeNotMounted("RPI-RP2")))
    }

    /// The bug this pair guards: a detection failure must not latch.  A drive
    /// that shows up late has to recover on its own, or the window stays stuck
    /// on an error while the drive sits mounted in Finder.
    func testLateArrivingDriveRecoversFromTheFailure() {
        let clock = TestClock()
        let locator = FakeBootloaderLocator(boards: [BootloaderBoard(chip: .rp2350, volumeURL: nil)])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { _ in throw FirmwareInstallError.noBoardFound },
                                          now: { clock.now })
        installer.beginWatching()
        clock.advance(FirmwareInstaller.volumeWaitTimeout)
        locator.emit()
        XCTAssertEqual(installer.state, .failed(.volumeNotMounted("RP2350")))

        let mounted = BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        locator.boards = [mounted]
        locator.emit()
        XCTAssertEqual(installer.state, .ready(mounted))
    }

    /// Unplugging a board must clear the wait, so plugging a second one in
    /// starts its own grace period rather than inheriting an expired one.
    func testRemovingTheBoardResetsTheVolumeWait() {
        let clock = TestClock()
        let locator = FakeBootloaderLocator(boards: [BootloaderBoard(chip: .rp2040, volumeURL: nil)])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { _ in throw FirmwareInstallError.noBoardFound },
                                          now: { clock.now })
        installer.beginWatching()
        clock.advance(FirmwareInstaller.volumeWaitTimeout - 1)

        locator.boards = []
        locator.emit()
        XCTAssertEqual(installer.state, .waitingForBoard)

        locator.boards = [BootloaderBoard(chip: .rp2040, volumeURL: nil)]
        locator.emit()
        XCTAssertEqual(installer.state, .waitingForVolume(.rp2040))
    }

    /// Two boards must be refused outright rather than resolved by guessing;
    /// writing to the wrong one is not recoverable from the app.
    func testTwoBoardsAreRefused() {
        let installer = makeInstaller(boards: [
            BootloaderBoard(chip: .rp2040, volumeURL: URL(fileURLWithPath: "/Volumes/RPI-RP2")),
            BootloaderBoard(chip: .rp2350, volumeURL: nil),
        ])
        installer.beginWatching()
        XCTAssertEqual(installer.state, .failed(.multipleBoards(2)))
    }

    /// Once a write has begun the board vanishing IS the expected reboot, so
    /// detection has to stop touching the state.  Before a write, detection
    /// stays live - that asymmetry is the whole fix.
    func testDetectionDoesNotDisturbAWriteInProgress() {
        let locator = FakeBootloaderLocator(boards: [
            BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        ])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { _ in throw FirmwareInstallError.noBoardFound })
        installer.beginWatching()
        installer.setStateForTesting(.writing(0.5), installing: true)
        locator.boards = []
        locator.emit()
        XCTAssertEqual(installer.state, .writing(0.5))
    }

    // MARK: - Committing to an update

    /// The hang this guards: with the board plugged in first, clicking Update
    /// changes no state, so anything that waits for `.ready` to *arrive* never
    /// fires and the window sits there forever.
    func testCommittingWithABoardAlreadyReadyStartsTheWrite() throws {
        let f = try SWM2Fixture(self)   // -- WP SW-M2 -- a fake drive, written by the core
        let board = f.board
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        XCTAssertEqual(installer.state, .ready(board))

        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)
        XCTAssertEqual(installer.state, .verified(FirmwareVersion(1, 1, 7)))
    }

    /// The other order: commit first, board arrives later.
    func testCommittingBeforeTheBoardArrivesStartsTheWriteWhenItDoes() throws {
        let f = try SWM2Fixture(self)
        let locator = f.locator
        locator.boards = []
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()

        installer.installWhenReady(confirmed: .firmwareFlash())
        XCTAssertEqual(installer.state, .waitingForBoard)

        locator.boards = [f.board]
        locator.emit()
        waitForSettledState(installer)
        XCTAssertEqual(installer.state, .verified(FirmwareVersion(1, 1, 7)))
    }

    /// Arming is a decision about this update, not standing permission: a
    /// board that turns up with nothing committed is never written to.
    func testAnUncommittedBoardIsNeverWritten() {
        let board = BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        let installer = makeInstaller(boards: [board])
        installer.beginWatching()
        XCTAssertEqual(installer.state, .ready(board))
        XCTAssertFalse(installer.isArmed)
    }

    // MARK: - A flash only counts when the device comes back

    func testSuccessRequiresTheDeviceToReturnRunningWhatWeWrote() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)

        XCTAssertEqual(installer.state, .verified(FirmwareVersion(1, 1, 7)))
        XCTAssertTrue(FileManager.default.fileExists(atPath: f.written.path))
    }

    /// A clean copy that produces no working device is a failure. Only the
    /// re-enumeration can tell the two apart.
    func testDeviceNeverReturningIsAFailure() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.stockBoard, verified: nil)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)

        XCTAssertEqual(installer.state, .failed(.deviceDidNotReturn))
    }

    func testDeviceReturningOnTheWrongVersionIsAFailure() throws {
        let f = try SWM2Fixture(self)
        let installer = f.installer(board: SWM2Fixture.stockBoard, verified: FirmwareVersion(1, 1, 6))
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)

        XCTAssertEqual(installer.state, .failed(.versionMismatch(expected: "1.1.7", got: "1.1.6")))
    }

    /// -- WP SW-M2 -- The write is reachable only through a person's
    /// confirmation: a ready board with nothing armed is never written, and a
    /// board without a drive never becomes ready to write.
    func testABoardWithoutAVolumeIsNeverWritten() {
        let installer = makeInstaller(boards: [BootloaderBoard(chip: .rp2040, volumeURL: nil)])
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        SWM2Fixture.spin(0.3) { false }
        XCTAssertEqual(installer.state, .waitingForVolume(.rp2040))
    }

    /// A release that bumped the app version and forgot the images must fail
    /// loudly here rather than shipping and downgrading every device it meets.
    func testStaleBundledImageIsRefused() throws {
        let f = try SWM2Fixture(self)
        let installer = FirmwareInstaller(
            locator: f.locator,
            verifier: StubVerifier(version: nil),
            imageProvider: { _ in
                throw FirmwareInstallError.bundledImageStale(bundled: "1.1.6", expected: "1.1.7")
            },
            gate: { nil })
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        SWM2Fixture.spin { if case .failed = installer.state { return true }; return false }
        XCTAssertEqual(installer.state, .failed(.bundledImageStale(bundled: "1.1.6", expected: "1.1.7")))
        XCTAssertFalse(FileManager.default.fileExists(atPath: f.written.path))
    }

    // MARK: - Life after an install

    /// The outcome of a flash has to stay on screen long enough to be read:
    /// the board rebooting out of BOOTSEL right after a success is the normal
    /// ending, not a change that may erase the green tick.
    func testVerifiedStateSurvivesTheBoardDisappearing() throws {
        let f = try SWM2Fixture(self)
        let locator = f.locator
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)
        XCTAssertEqual(installer.state, .verified(FirmwareVersion(1, 1, 7)))

        locator.boards = []
        locator.emit()
        XCTAssertEqual(installer.state, .verified(FirmwareVersion(1, 1, 7)))
    }

    /// The reported hang: a finished install froze detection for the life of
    /// the installer, so the window sat on its terminal state ignoring every
    /// plug and unplug. `reset()` is the door back to detection.
    func testResetAfterAVerifiedInstallReturnsToDetection() throws {
        let f = try SWM2Fixture(self)
        let locator = f.locator
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)

        locator.boards = []
        installer.reset()
        XCTAssertEqual(installer.state, .waitingForBoard)
    }

    /// Arming must not outlive the write it authorised: a second board seen
    /// after a reset was decided about exactly zero times, so it may become
    /// ready but never start writing on its own.
    func testASecondBoardAfterResetNeedsAFreshCommit() throws {
        let f = try SWM2Fixture(self)
        let locator = f.locator
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)
        installer.reset()

        let second = BootloaderBoard(chip: .rp2040, volumeURL: URL(fileURLWithPath: "/Volumes/RPI-RP2"))
        locator.boards = [second]
        locator.emit()
        XCTAssertEqual(installer.state, .ready(second))
        XCTAssertFalse(installer.isArmed)
    }

    /// A failure raised before any byte moved - here a missing image - used to
    /// last one poll tick: the board was still attached, so detection put
    /// `.ready` straight back over it, leaving a disabled button and no error.
    func testPreWriteFailureHoldsOverContinuedDetection() {
        let board = BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        let locator = FakeBootloaderLocator(boards: [board])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { chip in
                                              throw FirmwareInstallError.imageMissing(chip.displayName)
                                          },
                                          gate: { nil })
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        SWM2Fixture.spin { if case .failed = installer.state { return true }; return false }   // -- WP SW-M2 --
        XCTAssertEqual(installer.state, .failed(.imageMissing(BootloaderBoard.Chip.rp2350.displayName)))

        locator.emit()
        XCTAssertEqual(installer.state, .failed(.imageMissing(BootloaderBoard.Chip.rp2350.displayName)))

        installer.reset()
        XCTAssertEqual(installer.state, .ready(board))
        XCTAssertFalse(installer.isArmed)
    }

    /// Try Again after a genuine write failure has to actually start over;
    /// before `reset()` it restarted the watch against a permanently frozen
    /// state machine, which looked alive and did nothing.
    func testResetAfterAWriteFailureRecovers() throws {
        let f = try SWM2Fixture(self)
        let ghostVolume = URL(fileURLWithPath: "/nonexistent-\(UUID().uuidString)")
        let locator = f.locator
        locator.boards = [BootloaderBoard(chip: .rp2350, volumeURL: ghostVolume)]
        let installer = f.installer(board: SWM2Fixture.stockBoard)
        installer.beginWatching()
        installer.installWhenReady(confirmed: .firmwareFlash())
        waitForSettledState(installer)
        guard case .failed(.writeFailed) = installer.state else {
            return XCTFail("expected a write failure, got \(installer.state)")
        }

        let good = BootloaderBoard(chip: .rp2350, volumeURL: URL(fileURLWithPath: "/Volumes/RP2350"))
        locator.boards = [good]
        installer.reset()
        XCTAssertEqual(installer.state, .ready(good))
    }

    /// Mid-install there is no outcome to dismiss, and unfreezing detection
    /// would let the rebooting board's disappearance overwrite the write in
    /// progress - the exact bug the freeze exists to prevent.
    func testResetIsRefusedMidInstall() {
        let locator = FakeBootloaderLocator(boards: [])
        let installer = FirmwareInstaller(locator: locator,
                                          verifier: StubVerifier(version: nil),
                                          imageProvider: { _ in throw FirmwareInstallError.noBoardFound })
        installer.beginWatching()

        installer.setStateForTesting(.writing(0.5), installing: true)
        installer.reset()
        XCTAssertEqual(installer.state, .writing(0.5))

        installer.setStateForTesting(.waitingForDevice, installing: true)
        installer.reset()
        XCTAssertEqual(installer.state, .waitingForDevice)
        locator.emit()
        XCTAssertEqual(installer.state, .waitingForDevice)
    }

    // MARK: - The real bundle

    /// Proves the shipping app actually carries an image for each chip and
    /// that both match the app's own version. This is the release checklist
    /// enforced as a test: bump MARKETING_VERSION without refreshing the UF2s
    /// and this fails rather than shipping a downgrade.
    func testBundleCarriesMatchingImagesForBothChips() throws {
        for chip in BootloaderBoard.Chip.allCases {
            let image = try FirmwareImage.bundled(for: chip)
            XCTAssertEqual(image.version, FirmwareVersion.expected,
                           "bundled \(chip.assetToken) image is not the app's version")
            XCTAssertTrue(FileManager.default.fileExists(atPath: image.url.path))
        }
    }

    /// -- WP SW-M2 -- The shipping images pass the core's UF2 check for
    /// their own chip, and its guard: clear on a stock board, asked for the
    /// words on a board already in BOOTSEL (unknown) and on one that needs
    /// 1.20 V (today's images carry no voltage stamp).
    func testSWM2BundledImagesPassTheCoresCheckAndGuard() throws {
        for chip in BootloaderBoard.Chip.allCases {
            let bytes = try Data(contentsOf: try FirmwareImage.bundled(for: chip).url)
            let info = uf2Inspect(image: bytes)
            XCTAssertTrue(info.valid, info.problem)
            XCTAssertEqual(info.platform, chip.platform, chip.assetToken)
            XCTAssertTrue(firmwareGuardCheck(image: bytes, board: SWM2Fixture.stockBoard, typedConfirmation: "").allowed)
            for board in [firmwareGuardBoardInBootloader(), SWM2Fixture.atRiskBoard] {
                let verdict = firmwareGuardCheck(image: bytes, board: board, typedConfirmation: "")
                XCTAssertEqual(verdict.decision, .needsConfirmation, "\(chip.assetToken) \(board)")
                XCTAssertTrue(firmwareGuardCheck(image: bytes, board: board,
                                                 typedConfirmation: verdict.confirmationPhrase).allowed)
            }
        }
    }

    // MARK: - Helpers

    private func makeInstaller(boards: [BootloaderBoard],
                               verifier: FirmwareVerifying = StubVerifier(version: nil),
                               image: FirmwareImage? = nil) -> FirmwareInstaller {
        FirmwareInstaller(locator: FakeBootloaderLocator(boards: boards),
                          verifier: verifier,
                          imageProvider: { chip in
                              if let image { return image }
                              throw FirmwareInstallError.imageMissing(chip.displayName)
                          },
                          gate: { nil })
    }

    /// The install runs on a background queue and publishes to main; spin the
    /// run loop until it stops moving.
    private func waitForSettledState(_ installer: FirmwareInstaller) {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
            switch installer.state {
            case .verified, .failed: return
            default: continue
            }
        }
    }
}

// MARK: - Doubles

private struct StubVerifier: FirmwareVerifying {
    let version: FirmwareVersion?
    func awaitDeviceVersion(timeout: TimeInterval) -> FirmwareVersion? { version }
}

/// A clock the test moves by hand, so the volume-wait timeout is exercised
/// without any test actually waiting for it.
private final class TestClock {
    private(set) var now = Date(timeIntervalSince1970: 1_000_000)
    func advance(_ interval: TimeInterval) { now = now.addingTimeInterval(interval) }
}
