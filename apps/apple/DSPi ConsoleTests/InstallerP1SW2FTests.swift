import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP P1SW2F: the fixups of review p1sw2 (F1 the cover kept by a limit edit
/// right after the toggle, F3 the installer's lock over Swift's own firmware
/// copy, F4 the bootloader restart's device lock given back, F5 the
/// confirmation asked of every gated call, F7 `-UIMode` for one launch).  In
/// the macOS and the iOS bundles, on the core's demo device.
final class InstallerP1SW2FTests: XCTestCase, CoreDomainTesting {

    private var folders: [URL] = []

    override func tearDown() {
        for f in folders { try? FileManager.default.removeItem(at: f) }
        folders = []
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        CoreInstaller.launchMode = nil
        super.tearDown()
    }

    private func installer(_ vm: DSPViewModel) -> CoreInstaller {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("P1SW2F-\(UUID().uuidString)", isDirectory: true)
        folders.append(url)
        let i = CoreInstaller(vm: vm, folder: url)
        XCTAssertTrue(spinUntil(10) { i.available && !i.busy && i.choice != nil }, "the wrapper sees the core")
        return i
    }

    /// PIN set, the settings covered, locked now: the locked user.
    private func lockConsole(_ i: CoreInstaller) {
        i.setPin("1234", again: "1234")
        XCTAssertTrue(spinUntil(10) { i.lock.pinSet })
        i.setCovered(true)
        XCTAssertTrue(spinUntil(10) { !i.lock.covers.isEmpty && !i.busy })
        i.lockNow()
        XCTAssertTrue(spinUntil(10) { i.lockedUser && !i.busy })
    }

    // MARK: F1 (the reviewer's probe 2b, adopted)

    func testP1SW2FALimitEditRightAfterTheCoverToggleKeepsTheCover() {
        let (_, _, vm) = demoVM()
        let i = installer(vm)
        i.setPin("1234", again: "1234")
        XCTAssertTrue(spinUntil(10) { i.lock.pinSet && !i.busy })
        i.setCovered(true)
        // As soon as the lock says it is covered, before the profile is read
        // again: the draft follows at once.
        XCTAssertTrue(spinUntil(10) { !i.lock.covers.isEmpty })
        XCTAssertEqual(Set(i.draft.lockedPages), Set(i.lock.covers), "the draft carries the cover at once")
        i.setOutputLimit(0, "maxGainDb", -3)
        XCTAssertTrue(spinUntil(10) { !i.busy && i.status?.active == true })
        i.refresh()
        XCTAssertTrue(spinUntil(10) { !i.busy })
        XCTAssertFalse(i.lock.covers.isEmpty, "the edit kept what the lock covers")
        XCTAssertEqual(Set(i.lock.covers), Set(i.coverPages))
        XCTAssertFalse(i.draft.lockedPages.isEmpty)

        // Remove All Limits keeps the PIN and the cover too.
        i.clearLimits()
        XCTAssertTrue(spinUntil(10) { !i.busy })
        i.refresh()
        XCTAssertTrue(spinUntil(10) { !i.busy })
        XCTAssertTrue(i.lock.pinSet)
        XCTAssertEqual(Set(i.lock.covers), Set(i.coverPages), "the cover stays after Remove All Limits")
    }

    // MARK: F4

    func testP1SW2FARefusedRestartGivesTheDeviceLockBack() {
        let (_, _, vm) = demoVM()
        let i = installer(vm)
        lockConsole(i)
        var sent: Bool?
        vm.enterBootloaderForInstall(confirmed: .enterBootloader()) { sent = $0 }
        XCTAssertTrue(spinUntil(10) { sent != nil })
        XCTAssertEqual(sent, false, "the installer's lock refuses the restart (the core, TMCORE)")
        XCTAssertFalse(vm.firmwareInstallLock.held, "the lock taken for it is given back at once")
    }

    func testP1SW2FAnInstallsRestartHoldsTheCallersLockUntilReleased() {
        let (_, _, vm) = demoVM()
        var sent: Bool?
        vm.enterBootloaderForInstall(confirmed: .enterBootloader()) { sent = $0 }
        XCTAssertTrue(spinUntil(10) { sent != nil })
        XCTAssertEqual(sent, true)
        XCTAssertTrue(vm.firmwareInstallLock.held, "the install holds the lock it took (not the core's 60 s hold)")
        vm.releaseFirmwareInstallLock()
        XCTAssertFalse(vm.firmwareInstallLock.held, "and ends it when the install ends")
    }

    // MARK: F3

    func testP1SW2FTheFirmwareCopyIsRefusedWhileLocked() {
        let (_, _, vm) = demoVM()
        let i = installer(vm)
        XCTAssertNil(FirmwareFlashGate.refusal(i), "no PIN: nothing refused")
        lockConsole(i)
        let row = CoreInstaller.line(for: .InstallerLock(code: "locked", msg: ""))
        XCTAssertEqual(FirmwareFlashGate.refusal(i), .refused(row), "the core's row")

        let fw = FirmwareInstaller(locator: P1SW2FNoBoards(), verifier: P1SW2FNeverVerifier())
        fw.flashGate = { FirmwareFlashGate.refusal(i) }
        XCTAssertFalse(fw.installWhenReady(confirmed: .firmwareFlash()), "refused before anything is armed")
        XCTAssertEqual(fw.state, .failed(.refused(row)))
        XCTAssertFalse(fw.isArmed)
        XCTAssertEqual(FirmwareInstallError.refused(row).message, row)
    }

    func testP1SW2FAnInstallIsArmedOnlyByAFlashConfirmation() {
        let fw = FirmwareInstaller(locator: P1SW2FNoBoards(), verifier: P1SW2FNeverVerifier())
        XCTAssertTrue(fw.installWhenReady(confirmed: .firmwareFlash()))
        XCTAssertTrue(fw.isArmed)
        XCTAssertEqual(fw.flashConfirmation, .firmwareFlash())
    }

    // MARK: F5

    func testP1SW2FTheTokenComesFromThePersonsConfirmation() throws {
        let (core, t, _) = demoVM()
        // A confirmation mints a token for its own operation, which the gated
        // call takes; one for another operation is refused by the core.
        let token = t.coreQueue.sync { UserConfirmation.savePreset().token(core, for: .savePreset) }
        XCTAssertFalse(token.isEmpty)
        let r = t.call { try $0.savePresetConfirmed(slot: 2, token: token) }
        if case .failure(let e) = r { XCTFail("the token passes: \(e)") }
        let wrong = t.coreQueue.sync { core.confirmToken(op: UserConfirmation.copyPreset().op) }
        if case .success = t.call({ try $0.savePresetConfirmed(slot: 2, token: wrong) }) {
            XCTFail("a copy's confirmation does not save")
        }
    }

    // MARK: F7

    func testP1SW2FTheLaunchModeIsNotKept() {
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        CoreInstaller.launchMode = "simple"
        XCTAssertEqual(CoreInstaller.storedChoice(serial: "X"), "simple", "this launch")
        XCTAssertNil(UserDefaults.standard.string(forKey: CoreInstaller.modeKeyAll), "never written")
        let (_, _, vm) = demoVM()
        let i = installer(vm)
        i.choose("advanced", everyDevice: true)
        XCTAssertTrue(spinUntil(5) { i.mode == "advanced" })
        XCTAssertNil(CoreInstaller.launchMode, "a choice made in the app replaces it")
        XCTAssertEqual(CoreInstaller.storedChoice(serial: "X"), "advanced")
    }
}

private final class P1SW2FNoBoards: BootloaderLocating {
    func currentBoards() -> [BootloaderBoard] { [] }
    var onChange: (([BootloaderBoard]) -> Void)?
    func start() {}
    func stop() {}
}

private struct P1SW2FNeverVerifier: FirmwareVerifying {
    func awaitDeviceVersion(timeout: TimeInterval) -> FirmwareVersion? { nil }
}
