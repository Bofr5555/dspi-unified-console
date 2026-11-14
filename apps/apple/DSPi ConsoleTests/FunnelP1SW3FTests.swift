import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP P1SW3F (review p1sw3 F1, F2; from the reviewer's probe): what a person
/// is told when the installer's lock or read-only firmware refuses a preset,
/// flash or factory-reset call is the core's own row, not "Transfer failed";
/// and under the lock a recall asks no Save / Discard / Cancel question.  In
/// the macOS and the iOS bundles.
final class FunnelP1SW3FTests: XCTestCase, CoreDomainTesting {

    private var folders: [URL] = []

    override func tearDown() {
        for f in folders { try? FileManager.default.removeItem(at: f) }
        folders = []
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        super.tearDown()
    }

    private func lockedDemo() -> (CoreBackedTransport, DSPViewModel, CoreInstaller) {
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        let (_, t, vm) = demoVM()
        let folder = TestFolders.fresh("P1SW3F-limits")
        folders.append(folder)
        let installer = CoreInstaller(vm: vm, folder: folder)
        XCTAssertTrue(spinUntil(10) { installer.available && !installer.busy && installer.choice != nil })
        installer.setPin("1357", again: "1357")
        XCTAssertTrue(spinUntil(10) { installer.lock.pinSet })
        installer.setCovered(true)
        XCTAssertTrue(spinUntil(5) { !installer.lock.covers.isEmpty })
        installer.lockNow()
        XCTAssertTrue(spinUntil(10) { installer.lockedUser }, "locked")
        return (t, vm, installer)
    }

    private func unlock(_ installer: CoreInstaller) {
        var ok: Bool?
        installer.unlock("1357") { ok = $0 }
        XCTAssertTrue(spinUntil(10) { ok != nil })
    }

    private func isLockRow(_ key: String) -> Bool { key == "installer_lock" || key.hasPrefix("installer_lock.") }

    // MARK: - The six places (the probe)

    func testP1SW3FTheLocksRefusalsShowTheLocksRow() {
        let (_, vm, installer) = lockedDemo()
        defer { unlock(installer) }

        // 1. A preset save.
        let save = vm.savePresetOutcome(slot: 1, confirmed: .savePreset())
        XCTAssertNotEqual(save.status, PRESET_OK)
        let saveNotice = AppNotices.preset(save, "Failed to save preset")
        XCTAssertTrue(isLockRow(saveNotice.key), "the lock's row, not: \(saveNotice.line)")
        XCTAssertEqual(saveNotice.detail, "Failed to save preset")

        // 2. Factory Reset.
        let reset = vm.factoryResetOutcome(confirmed: .factoryReset())
        XCTAssertNotEqual(reset.status, FLASH_OK)
        let resetNotice = AppNotices.factoryReset(reset)
        XCTAssertTrue(isLockRow(resetNotice.key), "the lock's row, not: \(resetNotice.line)")

        // 3. Save Output Configuration.
        let oc = vm.saveOutputConfigOutcome(confirmed: .saveOutputConfig())
        guard case .failure(let failure) = oc.result else { return XCTFail("saved under the lock") }
        let ocNotice = AppNotices.outputConfig(failure, error: oc.error)
        XCTAssertTrue(isLockRow(ocNotice.key), "the lock's row, not: \(ocNotice.line)")
    }

    func testP1SW3FReadOnlyFirmwaresRefusalsShowTheReadOnlyRow() throws {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                       length: BULK_PARAMS_SIZE))
        let image = try XCTUnwrap(ReadOnlyFloorP1SW3Tests.fixtureImage(v32, wire: 33))
        let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: image), .fake))
        let vm = DSPViewModel(transport: core)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == core.generation })
        settle()
        XCTAssertTrue(vm.deviceReadOnly)

        // 4. A preset save, 5. Revert to Saved, 6. Factory Reset.
        let save = AppNotices.preset(vm.savePresetOutcome(slot: 1, confirmed: .savePreset()), "Failed to save preset")
        XCTAssertEqual(save.key, "read_only", save.line)
        let revert = AppNotices.revert(vm.loadParamsOutcome())
        XCTAssertEqual(revert.key, "read_only", revert.line)
        let reset = AppNotices.factoryReset(vm.factoryResetOutcome(confirmed: .factoryReset()))
        XCTAssertEqual(reset.key, "read_only", reset.line)
    }

    // MARK: - Each refusal its own row

    func testP1SW3FEachRefusalShowsItsOwnCoreRow() {
        let cases: [(DspiError, String)] = [
            (.Locked, "locked"),
            (.Denied, "denied"),
            (.ReadOnly, "read_only"),
            (.LimitExceeded(code: "limit_exceeded", msg: ""), "limit_exceeded"),
            (.ConfirmationRequired(code: "confirmation_required", msg: ""), "confirmation_required"),
        ]
        for (error, key) in cases {
            let status = DSPViewModel.presetStatus(error)
            XCTAssertEqual(AppNotices.preset(status, "Failed to save preset", error: error).key, key, "\(error)")
            XCTAssertEqual(AppNotices.revert(FLASH_ERR_WRITE, error: error).key, key, "\(error)")
            XCTAssertEqual(AppNotices.factoryReset(FLASH_ERR_WRITE, error: error).key, key, "\(error)")
            XCTAssertEqual(AppNotices.outputConfig(.error, error: error).key, key, "\(error)")
        }
        let lock = AppNotices.preset(0xFF, "Failed to save preset", error: .InstallerLock(code: "locked", msg: ""))
        XCTAssertTrue(isLockRow(lock.key), lock.key)
        XCTAssertNotEqual(AppNotices.preset(PRESET_STATUS_LOCKED, "x", error: .Denied).key, "locked",
                          "the hub's denial is not the hub's lock")
        // The device's own refusal byte and the raw path keep the status.
        XCTAssertEqual(AppNotices.preset(3, "Failed to save preset", error: .Refused(status: 3)).detail,
                       "Failed to save preset (error 3).")
        XCTAssertEqual(AppNotices.preset(CoreOutcome(3), "Failed to save preset").key, "transfer")
    }

    // MARK: - F2: a recall under the lock asks nothing

    func testP1SW3FARecallUnderTheLockAsksNothing() {
        XCTAssertTrue(ReadOnlyFloor.open.asksBeforeRecall(hasUnsavedChanges: true))
        XCTAssertFalse(ReadOnlyFloor.open.asksBeforeRecall(hasUnsavedChanges: false))

        let (t, vm, installer) = lockedDemo()
        defer { unlock(installer) }
        let floor = ReadOnlyFloor.of(vm, installer: installer)

        // A locked user's mute is an unsaved change ...
        vm.setOutputMute(output: 0, muted: true)
        XCTAssertTrue(spinUntil(5) { t.readState().outputMuted.first == true })
        vm.settleUnsavedChanges()
        XCTAssertTrue(spinUntil(5) { vm.hasUnsavedChanges }, "the mute is not in the preset")
        // ... that asks nothing before a recall, which the lock lets through.
        XCTAssertFalse(floor.asksBeforeRecall(hasUnsavedChanges: vm.hasUnsavedChanges))
        let other = vm.activePresetSlot == 0 ? 1 : 0
        let load = vm.loadPresetOutcome(slot: other)
        XCTAssertFalse(load.error.map { if case .InstallerLock = $0 { return true } else { return false } } ?? false,
                       "a recall is not refused by the lock: \(String(describing: load.error))")
    }

    // MARK: - The coordinator's two tweaks

    /// A device switch under the lock asks nothing either: its Save is the
    /// lock's refusal.  Unlocked, the same unsaved change asks.
    func testP1SW3FADeviceSwitchUnderTheLockAsksNothing() {
        let (t, vm, installer) = lockedDemo()
        vm.setOutputMute(output: 0, muted: true)
        XCTAssertTrue(spinUntil(5) { t.readState().outputMuted.first == true })
        vm.settleUnsavedChanges()
        XCTAssertTrue(spinUntil(5) { vm.hasUnsavedChanges })
        XCTAssertFalse(vm.asksBeforeDeviceSwitch(installer: installer), "locked: no Save / Discard / Cancel")
        unlock(installer)
        XCTAssertTrue(spinUntil(5) { !installer.lockedUser })
        vm.settleUnsavedChanges()
        XCTAssertTrue(spinUntil(5) { vm.asksBeforeDeviceSwitch(installer: installer) },
                      "unlocked: the unsaved mute asks before a switch")
    }

    /// On read-only firmware a module toggle's tool window still opens for
    /// reading, with its controls floored; under the lock only a window the
    /// lock allows opens.
    func testP1SW3FToolWindowsOpenForReadingOnReadOnlyFirmware() throws {
        let readOnly = ReadOnlyFloor(cause: .readOnlyFirmware)
        let locked = ReadOnlyFloor(cause: .installerLock)
        XCTAssertTrue(ReadOnlyFloor.open.opensToolWindow())
        XCTAssertTrue(readOnly.opensToolWindow(), "read-only firmware: open, for reading")
        XCTAssertFalse(readOnly.allows(.other), "... with its controls floored")
        XCTAssertFalse(locked.opensToolWindow(), "the lock: not a window it hides")
        XCTAssertTrue(locked.opensToolWindow(allowedByLock: true), "the lock: a window it leaves open")

        // On the V33 fixture: the floor is read-only firmware, and the tool
        // window's root builds over it.
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                       length: BULK_PARAMS_SIZE))
        let image = try XCTUnwrap(ReadOnlyFloorP1SW3Tests.fixtureImage(v32, wire: 33))
        let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: image), .fake))
        let vm = DSPViewModel(transport: core)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == core.generation })
        settle()
        let floor = ReadOnlyFloor.of(connected: vm.isDeviceConnected, deviceReadOnly: vm.deviceReadOnly, lockedUser: false)
        XCTAssertEqual(floor.cause, .readOnlyFirmware)
        XCTAssertTrue(floor.opensToolWindow())
        _ = CrossfeedView(vm: vm, crossfeed: vm.crossfeed).toolWindowFloor(vm)
    }
}
