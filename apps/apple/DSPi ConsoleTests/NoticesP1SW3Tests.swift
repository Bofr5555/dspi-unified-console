import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP P1SW3 (PRO1 U5): the notice host (`NoticeCenter`, `NoticeHost`) and the
/// alerts turned into notices.  Every failure is the core's notice row (P1NT)
/// with what failed as its detail; a success is a result line, never a
/// "Success" modal.  In the macOS and the iOS bundles.
final class NoticesP1SW3Tests: XCTestCase {

    private func row(_ key: String) -> Notice { noticeForKey(key: key, detail: "") }

    // MARK: - The notice host

    func testP1SW3TheHostShowsOneAtATimeInfoGoesByItselfWarningsStay() {
        let center = NoticeCenter(infoSeconds: 0.3)
        XCTAssertNil(center.current)

        center.post(.result("Filters exported"))
        XCTAssertEqual(center.current?.title, "Filters exported")
        XCTAssertEqual(center.current?.kind, .info)
        XCTAssertTrue(spinUntil(3) { center.current == nil }, "an info notice goes by itself")

        let warning = ConsoleNotice.row("locked", detail: "Output configuration not saved.")
        center.post(warning)
        settleFor(0.6)
        XCTAssertEqual(center.current, warning, "a warning stays until it is dismissed")

        // Behind it: the queue, the same notice not twice, at most maxWaiting.
        center.post(warning)
        XCTAssertTrue(center.waiting.isEmpty, "the notice on show is not queued again")
        for i in 0..<(NoticeCenter.maxWaiting + 2) { center.post(.result("r\(i)")) }
        XCTAssertEqual(center.waiting.count, NoticeCenter.maxWaiting)
        XCTAssertEqual(center.waiting.first?.title, "r2", "the oldest waiting are dropped")
        XCTAssertEqual(center.serial, 3 + NoticeCenter.maxWaiting + 2, "every post counts")

        center.dismiss()
        XCTAssertEqual(center.current?.title, "r2", "the next one follows a dismiss")
        center.clear()
        XCTAssertNil(center.current)
        XCTAssertTrue(center.waiting.isEmpty)
    }

    func testP1SW3APostFromAnotherThreadIsShownOnMain() {
        let center = NoticeCenter()
        DispatchQueue.global().async { center.post(AppNotices.notConnected) }
        XCTAssertTrue(spinUntil(3) { center.current != nil })
        XCTAssertEqual(center.current?.key, "disconnected")
        XCTAssertEqual(center.last, center.current)
    }

    func testP1SW3TheHostViewDrawsTheNoticeAndNothingWithout() {
        let center = NoticeCenter()
        XCTAssertEqual(NoticeHost.icon(.error), "xmark.octagon.fill")
        XCTAssertEqual(NoticeHost.icon(.warning), "exclamationmark.triangle.fill")
        center.post(AppNotices.file("No valid filters found in the file."))
        let n = center.current
        XCTAssertEqual(n?.title, row("io").title)
        XCTAssertEqual(n?.reason, row("io").reason)
        XCTAssertEqual(n?.action, row("io").action)
        XCTAssertEqual(n?.detail, "No valid filters found in the file.")
        XCTAssertEqual(n?.line, "\(row("io").title): \(row("io").reason) (No valid filters found in the file.)")
        // The view builds over both states.
        _ = NoticeHost(center: center).body
        center.clear()
        _ = NoticeHost(center: center).body
    }

    // MARK: - The core's rows

    func testP1SW3RowsAreTheCoresWithTheFamilyFallback() {
        for key in ["disconnected", "transfer", "io", "read_only", "locked", "config.not_read", "autoeq.network"] {
            let n = ConsoleNotice.row(key)
            XCTAssertEqual(n.key, key)
            XCTAssertEqual(n.title, row(key).title, key)
            XCTAssertEqual(n.reason, row(key).reason, key)
            XCTAssertFalse(n.title.isEmpty, key)
        }
        XCTAssertEqual(ConsoleNotice.row("autoeq.some_new_code").key, "autoeq", "a key the table lacks: its family's row")
        XCTAssertEqual(ConsoleNotice.row("installer_lock.not_a_row").key, "installer_lock")
        XCTAssertEqual(ConsoleNotice.row("no_such_key").key, "unknown")
        XCTAssertEqual(ConsoleNotice.row("transfer").kind, .error)
        XCTAssertEqual(ConsoleNotice.row("read_only").kind, .warning)
        XCTAssertEqual(ConsoleNotice.of(DspiError.Disconnected).key, "disconnected")
    }

    /// The wording the transports post for a refusal is the core's.
    func testP1SW3TheRefusalWordsAreTheCoresRows() {
        XCTAssertEqual(DeviceLock.lockedMessage, row("locked").title)
        XCTAssertEqual(DeviceLock.deniedMessage, "\(row("denied").title): \(row("denied").reason)")
        XCTAssertEqual(DeviceLock.readOnlyMessage, "\(row("read_only").title): \(row("read_only").reason)")
        XCTAssertEqual(RefusedWrite.unsupportedMessage, "\(row("unsupported").title): \(row("unsupported").reason)")
        XCTAssertEqual(RefusedWrite.transferMessage, "\(row("transfer").title): \(row("transfer").reason)")
    }

    // MARK: - The conversions (what each former alert says now)

    func testP1SW3ThePresetAndFlashFailuresAreTheCoresRows() {
        XCTAssertEqual(AppNotices.notConnected.key, "disconnected")
        XCTAssertEqual(AppNotices.stillReading.key, "config.not_read")
        XCTAssertEqual(AppNotices.readOnly.key, "read_only")

        // Save, copy, clear, load: the lock, or a transfer failure with the status.
        let locked = AppNotices.preset(PRESET_STATUS_LOCKED, "Failed to save preset")
        XCTAssertEqual(locked.key, "locked")
        XCTAssertEqual(locked.detail, "Failed to save preset")
        let failed = AppNotices.preset(3, "Failed to copy preset")
        XCTAssertEqual(failed.key, "transfer")
        XCTAssertEqual(failed.kind, .error)
        XCTAssertEqual(failed.detail, "Failed to copy preset (error 3).")
        XCTAssertEqual(AppNotices.presetLoad(PRESET_ERR_CRC).detail, "Preset data is corrupted.")
        XCTAssertEqual(AppNotices.presetLoad(PRESET_STATUS_LOCKED).key, "locked")
        XCTAssertEqual(AppNotices.presetLoad(0xFF).detail, "Failed to load preset (error 255).")

        // Revert to Saved and Factory Reset: a result line or a row.
        XCTAssertEqual(AppNotices.revert(FLASH_OK), .result("Parameters reverted"))
        XCTAssertEqual(AppNotices.revert(FLASH_ERR_NO_DATA).kind, .info)
        XCTAssertEqual(AppNotices.revert(FLASH_ERR_NO_DATA).key, "", "no core row: a status line")
        XCTAssertEqual(AppNotices.revert(FLASH_ERR_CRC).key, "transfer")
        XCTAssertEqual(AppNotices.revert(FLASH_ERR_WRITE).detail, "Failed to load parameters.")
        XCTAssertEqual(AppNotices.factoryReset(FLASH_OK), .result("Factory reset complete"))
        XCTAssertEqual(AppNotices.factoryReset(FLASH_ERR_WRITE).key, "transfer")
    }

    func testP1SW3TheFileFailuresAreTheCoresRows() {
        let err = NSError(domain: NSCocoaErrorDomain, code: NSFileReadNoSuchFileError,
                          userInfo: [NSLocalizedDescriptionKey: "The file does not exist."])
        let read = AppNotices.file(err)
        XCTAssertEqual(read.key, "io")
        XCTAssertEqual(read.detail, "The file does not exist.")
        XCTAssertEqual(AppNotices.configUnreadable(err).key, "config.unreadable")
        XCTAssertEqual(AppNotices.configUnreadable(DspiError.Io(msg: "bad json")).key, "io",
                       "a core error keeps the core's own row")
        XCTAssertEqual(AppNotices.locked("The configuration was not applied.").detail, "The configuration was not applied.")

        // The configuration export: the core's row, the old sentence as its detail.
        let export = FileMenuActions.configurationExportNotice(DspiError.Transfer(msg: "no settings have been read"))
        XCTAssertEqual(export.key, "transfer")
        XCTAssertTrue(export.detail.hasPrefix("Could not export the configuration"), export.detail)
        XCTAssertEqual(FileMenuActions.configurationExportNotice(err).key, "transfer")
    }

    func testP1SW3TheRefusalsBeforeAnImportOrExportAreRows() {
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        XCTAssertFalse(vm.isDeviceConnected)
        XCTAssertEqual(FileMenuActions.configurationExportRefusal(vm)?.key, "disconnected")
        #if os(iOS)
        XCTAssertEqual(FileMenuActions.importRefusal(vm, needsDevice: true)?.key, "disconnected")
        XCTAssertNil(FileMenuActions.importRefusal(vm, needsDevice: false))
        vm.deviceReadOnly = true
        XCTAssertEqual(FileMenuActions.importRefusal(vm, needsDevice: false)?.key, "read_only")
        #endif
    }

    func testP1SW3TheAutoEQOutcomesAreNotices() {
        let failed = AppNotices.autoEQ(DspiError.AutoEq(code: "network", msg: "offline", retryAfterS: nil))
        XCTAssertTrue(failed.key.hasPrefix("autoeq"), failed.key)
        XCTAssertEqual(failed.title, row(failed.key).title)
        XCTAssertEqual(AppNotices.autoEQ(NSError(domain: "x", code: 1)).key, "autoeq")

        func progress(succeeded: Bool, code: String) -> AutoEqRebuildProgress {
            AutoEqRebuildProgress(fraction: 1, status: "", finished: true, succeeded: succeeded, code: code,
                                  retryAfterS: nil, entries: 1, failedProfiles: 0, failedListings: 0,
                                  skippedProfiles: 0)
        }
        let done = AutoEQCoreRebuild.notice(progress(succeeded: true, code: ""))
        XCTAssertEqual(done.kind, .info)
        XCTAssertTrue(done.title.contains("Entries: 1"), done.title)
        XCTAssertFalse(done.title.contains("Success"), "no success heading: \(done.title)")
        XCTAssertEqual(AutoEQCoreRebuild.notice(progress(succeeded: false, code: "cancelled")).kind, .info)
        let net = AutoEQCoreRebuild.notice(progress(succeeded: false, code: "network"))
        XCTAssertEqual(net.key, "autoeq.network")
        XCTAssertTrue(net.detail.contains("The database was not changed."), net.detail)
        XCTAssertEqual(AutoEQCoreRebuild.notice(progress(succeeded: false, code: "zzz")).key, "autoeq")
    }

    /// Results carry no "Success" heading, and a report keeps its heading.
    func testP1SW3ResultsAreLinesNotSuccessModals() {
        for r in [AppNotices.revert(FLASH_OK), AppNotices.factoryReset(FLASH_OK), .result("Preset saved")] {
            XCTAssertEqual(r.kind, .info)
            XCTAssertEqual(r.key, "")
            XCTAssertFalse(r.title.localizedCaseInsensitiveContains("success"), r.title)
        }
        let report = ConsoleNotice.report("Import Device Configuration", "Skipped: ADAT", kind: .warning)
        XCTAssertEqual(report.title, "Import Device Configuration")
        XCTAssertEqual(report.reason, "Skipped: ADAT")
        XCTAssertEqual(report.kind, .warning)
    }

    #if os(iOS)
    /// End to end on iOS: a filter file that is not one posts the core's
    /// `io` row on the main window's host, with no alert.
    func testP1SW3AFilterImportFailurePostsANotice() {
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        NoticeCenter.main.clear()
        let before = NoticeCenter.main.serial
        FileMenuActions.importFilters("not a filter file at all", vm: vm)
        XCTAssertTrue(spinUntil(3) { NoticeCenter.main.serial > before })
        XCTAssertEqual(NoticeCenter.main.last?.key, "io")
        XCTAssertEqual(NoticeCenter.main.last?.detail, "No valid filters found in the file.")
        XCTAssertNil(UIApplication.shared.topViewController as? UIAlertController, "no modal")
        NoticeCenter.main.clear()
    }
    #endif

    private func settleFor(_ seconds: TimeInterval) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }
}
