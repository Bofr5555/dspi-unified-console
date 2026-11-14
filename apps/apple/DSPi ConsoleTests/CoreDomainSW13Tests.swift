import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW13: the presets and the configuration files through the core
/// (Commands.swift "Preset Commands", "Master Volume Mode", "Output Config
/// Mode", "Flash Storage Commands"; PresetDocumentTransfer.swift).  On the
/// core's demo device, and over Swift's FakeTransport under the core where a
/// test needs to see the device lock.  The core takes the lock for a save,
/// copy, load, delete, flash save, reset and import; the journal records
/// every preset edit, and Undo steps over them (the registry calls them
/// flash writes); a load, an import and a factory reset end the history.
final class CoreDomainSW13Tests: XCTestCase, CoreDomainTesting {

    /// `body` off main, as the views call the preset commands.
    private func offMain<T>(_ body: () -> T) -> T { DispatchQueue.global().sync(execute: body) }

    /// Every request this domain has on the wire, SET and GET, and the
    /// setters the Swift import used.
    private static let domainRequests: Set<UInt8> = [
        REQ_PRESET_SAVE, REQ_PRESET_LOAD, REQ_PRESET_DELETE, REQ_PRESET_SET_NAME, REQ_PRESET_GET_NAME,
        REQ_PRESET_GET_DIR, REQ_PRESET_GET_ACTIVE, REQ_PRESET_SET_STARTUP, REQ_PRESET_GET_STARTUP,
        REQ_SET_MASTER_VOLUME_MODE, REQ_GET_MASTER_VOLUME_MODE, REQ_SAVE_MASTER_VOLUME, REQ_GET_SAVED_MASTER_VOLUME,
        REQ_SET_OUTPUT_CONFIG_MODE, REQ_GET_OUTPUT_CONFIG_MODE, REQ_SAVE_OUTPUT_CONFIG, REQ_SAVE_PARAMS,
        REQ_FACTORY_RESET, REQ_SET_ALL_PARAMS, REQ_SET_PREAMP_CH, REQ_SET_EQ_PARAM,
    ]

    #if DEBUG
    private func domainRaw(_ t: CoreBackedTransport) -> [String] {
        t.takeRawOpcodesForTest().filter { Self.domainRequests.contains($0) }.map { String(format: "0x%02X", $0) }
    }
    #endif

    private func journalled(_ core: DspiCore, _ t: CoreBackedTransport, _ path: String,
                            file: StaticString = #filePath, line: UInt = #line) -> HistoryEntry? {
        let e = entries(core, t, path).last
        XCTAssertNotNil(e, "\(path) is in the journal", file: file, line: line)
        return e
    }

    private func flushed(_ t: CoreBackedTransport) -> DspState {
        flush(t)
        return t.readState()
    }

    private static let longName = "SW13 é preset with a name past thirty-one bytes"

    // MARK: - No raw requests in this domain

    /// Every command of the domain on the core's demo device: none of its
    /// requests goes out raw, each lands in the core's mirror and journal,
    /// and the view model shows what the core holds.
    func testSW13EveryPresetCallGoesThroughTheCore() throws {
        let (core, t, vm) = demoVM()
        #if DEBUG
        _ = t.takeRawOpcodesForTest()   // the connect-time read
        #endif

        XCTAssertEqual(offMain { vm.savePreset(slot: 4, confirmed: .savePreset()) }, PRESET_OK)
        XCTAssertTrue(spinUntil(5) { vm.activePresetSlot == 4 && vm.isPresetOccupied(4) && !vm.hasUnsavedChanges })
        XCTAssertFalse(try XCTUnwrap(journalled(core, t, "preset.save")).undoable)

        vm.setPresetName(slot: 4, name: Self.longName)
        let cut = CsCoreJSON.name(Self.longName)
        XCTAssertLessThanOrEqual(cut.utf8.count, 31)
        XCTAssertTrue(spinUntil(5) { vm.presetNames[4] == cut }, "shown as the core cuts it")
        XCTAssertEqual(flushed(t).presetNames[4], cut, "the core cut it the same way")
        XCTAssertFalse(try XCTUnwrap(journalled(core, t, "preset.name")).undoable)

        XCTAssertEqual(offMain { vm.copyPreset(from: 4, to: 5, confirmed: .copyPreset()) }, PRESET_OK)
        XCTAssertTrue(spinUntil(5) { vm.activePresetSlot == 4 && vm.isPresetOccupied(5) })
        XCTAssertEqual(t.readState().activePresetSlot, 4, "the source stays active")

        vm.setPresetStartup(mode: 0, defaultSlot: 5)
        vm.setOutputConfigMode(OUTPUT_CONFIG_MODE_INDEPENDENT)
        vm.setMasterVolumeMode(MASTER_VOLUME_MODE_WITH_PRESET)
        let s = flushed(t)
        XCTAssertEqual(s.presetStartupMode, 0)
        XCTAssertEqual(s.presetDefaultSlot, 5)
        XCTAssertEqual(Int(s.outputConfigMode), OUTPUT_CONFIG_MODE_INDEPENDENT)
        XCTAssertEqual(Int(s.masterVolumeMode), MASTER_VOLUME_MODE_WITH_PRESET)
        for path in ["preset.startup", "preset.iomode", "vol.master.mode"] {
            XCTAssertFalse(try XCTUnwrap(journalled(core, t, path)).undoable, path)
        }
        settle(0.7)   // past the pending window: the mirror's values are shown
        XCTAssertEqual(vm.presetStartupMode, 0)
        XCTAssertEqual(vm.presetDefaultSlot, 5)
        XCTAssertEqual(vm.presetOutputConfigMode, OUTPUT_CONFIG_MODE_INDEPENDENT)
        XCTAssertEqual(vm.presetMasterVolumeMode, MASTER_VOLUME_MODE_WITH_PRESET)

        XCTAssertTrue(offMain { vm.saveMasterVolume() })
        _ = journalled(core, t, "vol.master.save")
        guard case .success = offMain({ vm.saveOutputConfigResult(confirmed: .saveOutputConfig()) }) else { return XCTFail("output save") }
        _ = journalled(core, t, "dev.save.io")
        XCTAssertEqual(offMain { vm.saveParams(confirmed: .saveParams()) }, FLASH_OK)
        _ = journalled(core, t, "dev.save")
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no raw write in the journal")

        // Reads: the directory and a name again, the active slot from the mirror.
        let occupied = offMain { vm.fetchPresetDirectory() }
        XCTAssertEqual(occupied & 0b11_0000, 0b11_0000)
        vm.presetNames[5] = "stale"
        offMain { vm.fetchPresetName(slot: 5) }
        let name5 = t.readState().presetNames[5]
        XCTAssertTrue(spinUntil(5) { vm.presetNames[5] == name5 })
        vm.activePresetSlot = 9
        vm.fetchPresetActive()
        XCTAssertEqual(vm.activePresetSlot, 4, "the core's copy, no I/O, on main")

        XCTAssertEqual(offMain { vm.loadPreset(slot: 5) }, PRESET_OK)
        XCTAssertTrue(spinUntil(5) { vm.activePresetSlot == 5 })
        XCTAssertEqual(offMain { vm.deletePreset(slot: 5, confirmed: .deletePreset()) }, PRESET_OK)
        XCTAssertTrue(offMain { vm.waitForPresetDeletion(slot: 5) })
        XCTAssertTrue(spinUntil(5) { !vm.isPresetOccupied(5) })
        XCTAssertEqual(offMain { vm.factoryReset(confirmed: .factoryReset()) }, FLASH_OK)
        XCTAssertTrue(spinUntil(10) { vm.isDeviceConnected })

        #if DEBUG
        XCTAssertEqual(domainRaw(t), [], "no request of the domain went out raw")
        #endif
    }

    /// Export through the core is the core's document, which this app's codec
    /// reads; Import on another demo device through the core applies it in
    /// one bulk write, with the core's report, and nothing goes out raw.
    func testSW13ExportAndImportGoThroughTheCore() throws {
        let (_, _, source) = demoVM()
        source.setFilter(ch: 0, band: 3, p: FilterParams(type: .peaking, freq: 3150, q: 2, gain: 5))
        source.setPreampChannel(channel: 1, db: -2)
        settle(0.6)

        var made: Result<PresetDocumentExport.File, Error>?
        PresetDocumentExport.make(from: source, name: "SW13") { made = $0 }
        XCTAssertTrue(spinUntil(10) { made != nil })
        let file = try XCTUnwrap(made).get()
        let text = String(decoding: file.data, as: UTF8.self)
        XCTAssertEqual(file.doc.meta.name, "SW13")
        XCTAssertEqual(file.doc.channels[0].eq[3].freqHz, 3150, accuracy: 0.5, "this app's codec reads the core's file")

        let (core, t, target) = demoVM()
        target.setMasterVolume(target.masterVolumeDB - 3)
        XCTAssertTrue(spinUntil(5) { target.undo.canUndo })
        #if DEBUG
        _ = t.takeRawOpcodesForTest()
        #endif
        var report: PresetApplyReport?
        PresetDocumentApply.apply(file.doc, to: target, text: text, options: PresetApplyOptions()) { report = $0 }
        XCTAssertTrue(spinUntil(30) { report != nil }, "the import finishes")
        let r = try XCTUnwrap(report)
        XCTAssertFalse(r.lockedOut)
        XCTAssertTrue(r.isClean, "\(r.coreLines ?? [])")
        XCTAssertEqual(r.coreLines?.last, "These changes are live but not yet stored on the device. "
                       + "Save them to a preset slot to keep them.")
        XCTAssertEqual(FileMenuActions.configurationImportResultText(r), r.coreLines?.joined(separator: "\n\n"))
        XCTAssertGreaterThan(r.channelsApplied, 0)
        XCTAssertGreaterThan(r.bandsApplied, 0)

        XCTAssertTrue(spinUntil(5) { target.channelData[0]?[3].freq == 3150 }, "the view model shows the import")
        XCTAssertEqual(target.preampValue(1), -2, accuracy: 0.05)
        XCTAssertEqual(entries(core, t, "bulk").count, 1, "one bulk write")
        XCTAssertTrue(spinUntil(5) { !target.undo.canUndo }, "the import ended the undo history")
        #if DEBUG
        XCTAssertEqual(domainRaw(t), [], "the import sent nothing raw")
        #endif
    }

    /// The options map onto the core's bits: audio always, volumes and the
    /// hardware I/O when chosen.  A refusal reply is a skipped line; the
    /// lock's is `lockedOut`.
    func testSW13ImportOptionsAndRepliesMapOntoTheReport() {
        XCTAssertEqual(PresetDocumentApply.coreOptions(PresetApplyOptions()), 0)
        XCTAssertEqual(PresetDocumentApply.coreOptions(PresetApplyOptions(audioProcessing: true, volumeLevels: true,
                                                                          hardwareIO: true)),
                       DspiConstants.IMPORT_VOLUMES | DspiConstants.IMPORT_HARDWARE)
        let locked = PresetDocumentApply.report(fromCoreReply: #"{"ok":false,"error":"Another console","noticeKey":"locked"}"#)
        XCTAssertTrue(locked.0.lockedOut)
        let unreadable = PresetDocumentApply.report(fromCoreReply: #"{"ok":false,"error":"not a document"}"#)
        XCTAssertFalse(unreadable.0.lockedOut)
        XCTAssertEqual(unreadable.0.skipped, ["not a document"])
        XCTAssertFalse(unreadable.0.isClean)
        let ok = PresetDocumentApply.report(fromCoreReply: """
            {"ok":true,"error":"","lines":["Applied 3 channels, 7 EQ bands, 2 crossover bands, 4 crosspoints.",
            "Not present on this device: Out 9, Out 10","Skipped: Upmixer (not on this platform)","Done."],
            "clean":false,"links":5}
            """)
        XCTAssertEqual([ok.0.channelsApplied, ok.0.bandsApplied, ok.0.crossoverBandsApplied, ok.0.crosspointsApplied],
                       [3, 7, 2, 4])
        XCTAssertEqual(ok.0.missingChannels, ["Out 9", "Out 10"])
        XCTAssertEqual(ok.0.skipped, ["Upmixer (not on this platform)"])
        XCTAssertEqual(ok.links, 5)
        XCTAssertFalse(ok.0.isClean)
    }

    /// The shared fixtures (the Mac's and the Qt console's files) import
    /// through the core with the file's own text, and the core's export of
    /// the result is read by this app's codec as the same channels.
    func testSW13SharedFixturesThroughTheCore() throws {
        let fixtures = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("tests/fixtures/preset")
        for name in ["mac-swift-v32.dspipreset", "qt-core-v32.dspipreset"] {
            let data = try Data(contentsOf: fixtures.appendingPathComponent(name))
            let doc = try PresetDocumentFile.decode(data)
            let (_, _, vm) = demoVM()
            var report: PresetApplyReport?
            PresetDocumentApply.apply(doc, to: vm, text: String(decoding: data, as: UTF8.self),
                                      options: PresetApplyOptions(audioProcessing: true, volumeLevels: false,
                                                                  hardwareIO: true)) { report = $0 }
            XCTAssertTrue(spinUntil(30) { report != nil }, name)
            let r = try XCTUnwrap(report, name)
            XCTAssertFalse(r.lockedOut, name)
            XCTAssertTrue(r.missingChannels.isEmpty, "\(name): \(r.coreLines ?? [])")
            var made: Result<PresetDocumentExport.File, Error>?
            PresetDocumentExport.make(from: vm, name: name) { made = $0 }
            XCTAssertTrue(spinUntil(10) { made != nil }, name)
            let back = try XCTUnwrap(made, name).get().doc
            XCTAssertEqual(back.channels.count, doc.channels.count, name)
            for (a, b) in zip(back.channels, doc.channels) {
                XCTAssertEqual(a.channelId, b.channelId, name)
                XCTAssertEqual(a.eq.map(\.type), b.eq.map(\.type), "\(name) channel \(a.channelId)")
            }
        }
    }

    // MARK: - The device lock

    /// The core's lock calls as the host sees them, and every request with
    /// whether the lock was held when it went out.  `refuse`: another
    /// console holds the lock.
    final class LockTransport: ForwardingTransport {
        private let stateLock = NSLock()
        private var held = 0
        private var log: [(UInt8, Bool)] = []
        var refuse = false
        private(set) var acquired = 0

        override func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool {
            stateLock.lock(); defer { stateLock.unlock() }
            guard !refuse else { return false }
            held += 1
            acquired += 1
            return true
        }
        override func releaseDeviceLock() {
            stateLock.lock(); held = max(0, held - 1); stateLock.unlock()
        }
        private func note(_ request: UInt8) {
            stateLock.lock(); log.append((request, held > 0)); stateLock.unlock()
        }
        override func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
            note(request)
            super.sendControlRequest(request: request, value: value, index: index, data: data)
        }
        override func getControlResult(request: UInt8, value: UInt16, index: UInt16,
                                       length: UInt16) -> Result<Data, DSPi_Console.LinkStatus> {
            note(request)
            return super.getControlResult(request: request, value: value, index: index, length: length)
        }
        var isHeld: Bool { stateLock.lock(); defer { stateLock.unlock() }; return held > 0 }
        /// Whether each `request` sent since the last call went out under the lock.
        func take(_ request: UInt8) -> [Bool] {
            stateLock.lock(); defer { log = []; stateLock.unlock() }
            return log.filter { $0.0 == request }.map(\.1)
        }
        func clear() { stateLock.lock(); log = []; stateLock.unlock() }
    }

    private func lockedVM() -> (LockTransport, CoreBackedTransport, DSPViewModel) {
        let fake = FakeTransport(autoSelect: false)
        let host = LockTransport(fake)
        let t = CoreBackedTransport(link: .host(host, .fake), core: DspiCore())
        let vm = DSPViewModel(transport: t)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        settle()
        host.clear()
        return (host, t, vm)
    }

    private func importDocument(_ vm: DSPViewModel) -> PresetApplyReport? {
        let doc = PresetDocument.capture(from: vm)
        var report: PresetApplyReport?
        PresetDocumentApply.apply(doc, to: vm, options: PresetApplyOptions()) { report = $0 }
        XCTAssertTrue(spinUntil(30) { report != nil }, "the import finishes")
        return report
    }

    /// The preset-half of LinkLockUsageTests, through the core: each save,
    /// the copy (one lock across both saves), the flash save, the output
    /// configuration's save and the import send their requests under the
    /// lock the core takes, and give it back.
    func testSW13TheCoreHoldsTheLockForEachFlashWriteAndTheImport() throws {
        let (host, _, vm) = lockedVM()

        XCTAssertEqual(offMain { vm.savePreset(slot: 2, confirmed: .savePreset()) }, PRESET_OK)
        XCTAssertEqual(host.take(REQ_PRESET_SAVE), [true])
        XCTAssertFalse(host.isHeld)

        XCTAssertEqual(offMain { vm.copyPreset(from: 2, to: 6, confirmed: .copyPreset()) }, PRESET_OK)
        XCTAssertEqual(host.take(REQ_PRESET_SAVE), [true, true], "both saves under the one lock")
        XCTAssertFalse(host.isHeld)

        XCTAssertEqual(offMain { vm.saveParams(confirmed: .saveParams()) }, FLASH_OK)
        XCTAssertEqual(host.take(REQ_SAVE_PARAMS), [true])
        // Swift's fake does not answer this save (the demo device's test
        // covers its success): what counts here is the lock it went out under.
        _ = offMain { vm.saveOutputConfigResult(confirmed: .saveOutputConfig()) }
        XCTAssertEqual(host.take(REQ_SAVE_OUTPUT_CONFIG), [true])
        XCTAssertFalse(host.isHeld)

        XCTAssertEqual(offMain { vm.loadPreset(slot: 6) }, PRESET_OK)
        XCTAssertEqual(host.take(REQ_PRESET_LOAD), [true])
        XCTAssertEqual(offMain { vm.deletePreset(slot: 6, confirmed: .deletePreset()) }, PRESET_OK)
        XCTAssertEqual(host.take(REQ_PRESET_DELETE), [true])

        let report = try XCTUnwrap(importDocument(vm))
        XCTAssertFalse(report.lockedOut)
        XCTAssertEqual(host.take(REQ_SET_ALL_PARAMS), [true], "the image, whole, under the lock")
        XCTAssertFalse(host.isHeld)
    }

    /// Another console holds the lock: each command reads as the lock and
    /// sends nothing of its own.
    func testSW13ALockHeldElsewhereRefusesAndSendsNothing() throws {
        let (host, _, vm) = lockedVM()
        host.refuse = true

        XCTAssertEqual(offMain { vm.savePreset(slot: 2, confirmed: .savePreset()) }, PRESET_STATUS_LOCKED)
        XCTAssertEqual(presetFailureText(PRESET_STATUS_LOCKED), DeviceLock.lockedMessage)
        XCTAssertEqual(offMain { vm.copyPreset(from: 0, to: 6, confirmed: .copyPreset()) }, PRESET_STATUS_LOCKED)
        XCTAssertEqual(offMain { vm.loadPreset(slot: 0) }, PRESET_STATUS_LOCKED)
        XCTAssertEqual(offMain { vm.deletePreset(slot: 0, confirmed: .deletePreset()) }, PRESET_STATUS_LOCKED)
        XCTAssertEqual(offMain { vm.saveParams(confirmed: .saveParams()) }, FLASH_ERR_WRITE)
        guard case .failure(.locked) = offMain({ vm.saveOutputConfigResult(confirmed: .saveOutputConfig()) }) else {
            return XCTFail("the output save reads as the lock")
        }
        XCTAssertEqual(offMain { vm.factoryReset(confirmed: .factoryReset()) }, FLASH_ERR_WRITE)
        let report = try XCTUnwrap(importDocument(vm))
        XCTAssertTrue(report.lockedOut)

        for request in [REQ_PRESET_SAVE, REQ_PRESET_LOAD, REQ_PRESET_DELETE, REQ_SAVE_PARAMS,
                        REQ_SAVE_OUTPUT_CONFIG, REQ_FACTORY_RESET, REQ_SET_ALL_PARAMS] {
            XCTAssertEqual(host.take(request), [], String(format: "0x%02X was not sent", request))
        }
    }

    // MARK: - Undo (the core's registry)

    /// Every preset edit is a flash write: recorded, never undone.  Undo
    /// steps over them, says so, and undoes the change before them.
    func testSW13UndoStepsOverPresetEdits() throws {
        let (core, t, vm) = demoVM()
        let volume = vm.masterVolumeDB
        vm.setMasterVolume(volume - 6)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        let slot = vm.activePresetSlot
        vm.setPresetName(slot: slot, name: "SW13 undo")
        // Values other than the device's: a write that changes nothing is
        // not journalled.
        vm.setPresetStartup(mode: vm.presetStartupMode == 0 ? 1 : 0, defaultSlot: slot == 9 ? 8 : 9)
        let ioMode = vm.presetOutputConfigMode == OUTPUT_CONFIG_MODE_INDEPENDENT
            ? OUTPUT_CONFIG_MODE_WITH_PRESET : OUTPUT_CONFIG_MODE_INDEPENDENT
        vm.setOutputConfigMode(ioMode)
        for path in ["preset.name", "preset.startup", "preset.iomode"] {
            XCTAssertFalse(try XCTUnwrap(journalled(core, t, path)).undoable, path)
        }
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == volume }, "the volume change is undone")
        XCTAssertNotNil(vm.undo.notice, "the steps over are named")
        XCTAssertEqual(vm.presetNames[slot], "SW13 undo", "the preset edits stay")
        XCTAssertEqual(t.readState().presetNames[slot], "SW13 undo")
        XCTAssertEqual(Int(t.readState().outputConfigMode), ioMode)
    }

    /// A preset load and a factory reset end the history before them
    /// (an import's is in testSW13ExportAndImportGoThroughTheCore).
    func testSW13ALoadAndAFactoryResetEndTheHistory() {
        let (_, _, vm) = demoVM()
        XCTAssertEqual(offMain { vm.savePreset(slot: 3, confirmed: .savePreset()) }, PRESET_OK)

        vm.setMasterVolume(vm.masterVolumeDB - 4)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertEqual(offMain { vm.loadPreset(slot: 3) }, PRESET_OK)
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo }, "the load ended the history")

        vm.setMasterVolume(vm.masterVolumeDB - 4)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertEqual(offMain { vm.factoryReset(confirmed: .factoryReset()) }, FLASH_OK)
        XCTAssertTrue(spinUntil(10) { !vm.undo.canUndo }, "the reset ended the history")
    }
}
