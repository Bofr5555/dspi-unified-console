import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console
#if os(macOS)
import AppKit
#endif

/// WP P1SW1: snapshots, A/B, the import preview, the safe apply and offline
/// files through the core (Core/CoreSnapshots.swift, Snapshots/*.swift), on
/// the core's demo device.  In the macOS and the iOS bundles.
final class SnapshotsP1SW1Tests: XCTestCase, CoreDomainTesting {

    private var folders: [URL] = []

    override func tearDown() {
        for f in folders { try? FileManager.default.removeItem(at: f) }
        folders = []
        super.tearDown()
    }

    /// A fresh folder of the test's own (the VM's temporary directory).
    private func folder(_ name: String) -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("P1SW1-\(name)-\(UUID().uuidString)", isDirectory: true)
        folders.append(url)
        return url
    }

    /// The demo device, a view model on it and its snapshots wrapper, ready.
    private func setUpSnapshots() -> (DspiCore, CoreBackedTransport, DSPViewModel, CoreSnapshots) {
        let (core, t, vm) = demoVM()
        let snaps = CoreSnapshots(vm: vm, folder: folder("store"))
        XCTAssertTrue(spinUntil(10) { snaps.available && !snaps.busy }, "the wrapper sees the core")
        return (core, t, vm, snaps)
    }

    private func capture(_ snaps: CoreSnapshots, _ name: String, file: StaticString = #filePath,
                         line: UInt = #line) -> SnapshotInfo? {
        let before = snaps.snapshots.count
        let started = Date()
        var said: [String] = []
        let watch = snaps.$notice.compactMap { $0?.text }.sink { said.append($0) }
        defer { watch.cancel() }
        snaps.capture(name: name)
        let done = spinUntil(30) { snaps.snapshots.count == before + 1 && !snaps.busy }
        XCTAssertTrue(done, "captured \(name): \(snaps.snapshots.count) listed (\(before) before), busy \(snaps.busy), "
                      + "notices \(said), after \(Date().timeIntervalSince(started)) s",
                      file: file, line: line)
        return snaps.snapshots.first { $0.name == name }
    }

    private func export(_ t: CoreBackedTransport, _ name: String) throws -> String {
        try t.call { try $0.configExport(name: name, appVersion: "P1SW1 test", links: 0) }.get()
    }

    // MARK: - The store

    func testP1SW1CaptureRenameNoteExportImportDelete() throws {
        let (_, _, _, snaps) = setUpSnapshots()
        XCTAssertTrue(snaps.snapshots.isEmpty, "a new store is empty")
        let s = try XCTUnwrap(capture(snaps, "P1SW1 first"))
        XCTAssertEqual(snaps.notice?.text, "Captured \u{201C}P1SW1 first\u{201D}.", "a notice, no modal")

        snaps.rename(s, to: "  P1SW1 renamed é  ")
        XCTAssertTrue(spinUntil(5) { snaps.snapshots.first?.name == "P1SW1 renamed é" })
        snaps.setNote(try XCTUnwrap(snaps.snapshots.first), "a note")
        XCTAssertTrue(spinUntil(5) { snaps.snapshots.first?.note == "a note" })

        var exported: String?
        snaps.exportText(try XCTUnwrap(snaps.snapshots.first)) { exported = $0 }
        XCTAssertTrue(spinUntil(5) { exported != nil })
        let text = try XCTUnwrap(exported)
        XCTAssertTrue(text.contains("\"snapshot\""), "the snapshot's document")

        snaps.importText(text)
        XCTAssertTrue(spinUntil(5) { snaps.snapshots.count == 2 }, "filed again from its file")

        snaps.delete(try XCTUnwrap(snaps.snapshots.last))
        XCTAssertTrue(spinUntil(5) { snaps.snapshots.count == 1 && !snaps.busy })

        snaps.importText("{\"not\": \"a document\"}")
        XCTAssertTrue(spinUntil(5) { snaps.notice?.kind == .error }, "a refusal is a notice line")
        XCTAssertEqual(snaps.snapshots.count, 1)
    }

    // MARK: - Recall

    func testP1SW1RecallPutsTheSettingsBackAndEndsTheUndoHistory() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        let before = vm.masterVolumeDB
        let s = try XCTUnwrap(capture(snaps, "P1SW1 recall"))
        vm.setMasterVolume(before + 6)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo }, "the edit is undoable")
        flush(t)

        snaps.recall(s)
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before && !snaps.busy }, "recalled: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo }, "a recall ends the undo history")
        XCTAssertTrue(snaps.notice?.text.hasPrefix("Recalled \u{201C}P1SW1 recall\u{201D}") ?? false,
                      "the notice line: \(snaps.notice?.text ?? "none")")
        XCTAssertEqual(snaps.guardState.phase, .idle, "a plain recall on USB is not guarded")
    }

    func testP1SW1GuardedRecallRevertAndKeep() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        XCTAssertFalse(snaps.guardDefault, "on USB (the demo device) the guard is the user's choice")
        let before = vm.masterVolumeDB
        let s = try XCTUnwrap(capture(snaps, "P1SW1 guarded"))
        vm.setMasterVolume(before + 6)
        flush(t)
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == before + 6 })

        snaps.rollback = true
        snaps.timeoutS = 60
        snaps.recall(s)
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .waiting }, "waits for Keep")
        XCTAssertTrue(snaps.barShown)
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before }, "the snapshot is on the device")
        let line = snaps.barLines.1
        XCTAssertTrue(line.contains(" s "), "the countdown sentence: \(line)")
        XCTAssertFalse(line.contains("{s}"))

        snaps.recall(s)
        XCTAssertEqual(snaps.notice?.text, CoreSnapshots.finishFirst, "no second apply while one waits")

        snaps.revertChange()
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .reverted }, "\(snaps.guardState.phase)")
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before + 6 }, "the settings from before are back")
        XCTAssertTrue(snaps.barShown, "the end's sentence stays until OK")
        snaps.dismissEnd()
        XCTAssertFalse(snaps.barShown)

        snaps.recall(s)
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .waiting })
        snaps.keepChange()
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .kept })
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before }, "kept")
    }

    func testP1SW1GuardRevertsAtTheTimeout() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        let before = vm.masterVolumeDB
        let s = try XCTUnwrap(capture(snaps, "P1SW1 timeout"))
        vm.setMasterVolume(before - 6)
        flush(t)
        snaps.timeoutS = 1
        XCTAssertEqual(snaps.timeoutS, CoreSnapshots.timeoutRange.lowerBound, "5 s at least")
        snaps.timeoutS = 1000
        XCTAssertEqual(snaps.timeoutS, CoreSnapshots.timeoutRange.upperBound, "300 s at most")
        snaps.timeoutS = 5
        snaps.rollback = true
        snaps.recall(s)
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .waiting })
        XCTAssertTrue(spinUntil(15) { snaps.guardState.phase == .reverted }, "no answer: reverted by the poll")
        XCTAssertEqual(snaps.guardState.reason, "timeout")
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before - 6 })
    }

    // MARK: - A/B

    func testP1SW1CompareTogglesDiffsAndStopsForAGuard() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        let v0 = vm.masterVolumeDB
        let a = try XCTUnwrap(capture(snaps, "P1SW1 A"))
        vm.setMasterVolume(v0 + 6)
        flush(t)
        let b = try XCTUnwrap(capture(snaps, "P1SW1 B"))

        snaps.startCompare(aId: a.id, bId: b.id)
        XCTAssertTrue(spinUntil(10) { snaps.compare.active && snaps.compare.side == .a })
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == v0 }, "on A")
        XCTAssertTrue(spinUntil(10) { !snaps.abRows.isEmpty }, "the diff between A and B")
        XCTAssertTrue(snaps.canToggle)

        snaps.toggle()
        XCTAssertTrue(spinUntil(10) { snaps.compare.side == .b && !snaps.busy })
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == v0 + 6 }, "on B")
        XCTAssertEqual(snaps.guardState.phase, .idle, "a toggle is never guarded")

        // A guarded recall: the toggle is off; a revert ends the compare.
        snaps.rollback = true
        snaps.timeoutS = 60
        snaps.recall(a)
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .waiting })
        XCTAssertFalse(snaps.canToggle, "off during the countdown")
        snaps.toggle()
        XCTAssertEqual(snaps.notice?.text, CoreSnapshots.finishFirst)
        snaps.revertChange()
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .reverted })
        XCTAssertTrue(spinUntil(10) { !snaps.compare.active }, "a revert ends the compare")
    }

    func testP1SW1CompareKeepEndsIt() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        let v0 = vm.masterVolumeDB
        vm.setMasterVolume(v0 - 3)
        flush(t)
        let b = try XCTUnwrap(capture(snaps, "P1SW1 keep B"))
        vm.setMasterVolume(v0)
        flush(t)
        snaps.startCompare(aId: "", bId: b.id)   // A: the device now
        XCTAssertTrue(spinUntil(10) { snaps.compare.active && snaps.compare.aName == "A" })
        snaps.keep(side: .b)
        XCTAssertTrue(spinUntil(10) { !snaps.compare.active && !snaps.busy })
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == v0 - 3 }, "B kept")
    }

    // MARK: - The import preview

    func testP1SW1ImportPreviewChoosesBlocksAndAppliesWithAndWithoutRollback() throws {
        let (_, t, vm, snaps) = setUpSnapshots()
        let before = vm.masterVolumeDB
        let text = try export(t, "P1SW1 import")
        vm.setMasterVolume(before + 6)
        flush(t)

        let json = try t.call {
            $0.configPreview(text: text, options: ImportBlock.options(Set(ImportBlock.allCases)),
                             eqChannels: allEqChannels)
        }.get()
        let diff = SnapshotDiff.parse(json)
        XCTAssertTrue(diff.ok, diff.error)
        XCTAssertFalse(diff.rows.isEmpty, "the volume differs")
        let model = ImportPreviewModel(text: text, name: "P1SW1 import", intro: "", diff: diff, guardDefault: false)
        XCTAssertFalse(model.selected.contains(.volumes), "volume levels are opt-in")
        XCTAssertFalse(model.selected.contains(.hardware), "hardware I/O is opt-in")
        XCTAssertFalse(model.withRollback)

        model.selected = Set(ImportBlock.allCases)
        var applied: Bool?
        ImportPreview.apply(model, vm: vm, snaps: snaps) { applied = $0 }
        XCTAssertTrue(spinUntil(10) { applied != nil })
        XCTAssertEqual(applied, true, snaps.notice?.text ?? "")
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before }, "imported, the volume included")
        XCTAssertNotNil(snaps.notice, "the result is a notice line")

        vm.setMasterVolume(before + 3)
        flush(t)
        model.guarded = true
        snaps.timeoutS = 60
        applied = nil
        ImportPreview.apply(model, vm: vm, snaps: snaps) { applied = $0 }
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .waiting }, "Apply with rollback waits")
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before })
        snaps.revertChange()
        XCTAssertTrue(spinUntil(10) { snaps.guardState.phase == .reverted })
        XCTAssertTrue(spinUntil(10) { vm.masterVolumeDB == before + 3 }, "reverted")

        model.selected = []
        XCTAssertTrue(model.countLine.hasSuffix("0 changes chosen."))
    }

    // MARK: - Offline files and templates

    func testP1SW1OpensAFileOfflineSavesItAndATemplate() throws {
        let (_, t0, _) = demoVM()
        let text = try export(t0, "P1SW1 offline")
        let doc = CoreBackedTransport(link: .document)
        XCTAssertTrue(doc.isDocumentLink)
        XCTAssertTrue(doc.availableDevices.isEmpty, "nothing listed before a file")
        let vm = DSPViewModel(transport: doc)
        let snaps = CoreSnapshots(vm: vm, folder: folder("offline-store"))
        OfflineFiles.open(text: text, fileName: "P1SW1.dspipreset", vm: vm, transport: doc, snaps: snaps)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == doc.generation },
                      "the document opens as a device")
        XCTAssertEqual(doc.availableDevices.first?.displayName, "File: P1SW1.dspipreset")
        XCTAssertTrue(spinUntil(10) { snaps.offline?.badge == "File: P1SW1.dspipreset" }, "the badge")
        XCTAssertTrue(spinUntil(5) { snaps.notice?.text.hasPrefix("Opened") ?? false }, snaps.notice?.text ?? "")

        let saved = try doc.call { try $0.saveDocument(appVersion: "P1SW1 test", links: 0) }.get()
        XCTAssertTrue(saved.contains("P1SW1 offline"), "the document as it stands, under its name")

        let templates = folder("templates")
        var ok: Bool?
        OfflineFiles.saveTemplate(name: "P1SW1 template", vm: vm, snaps: snaps, folder: templates) { ok = $0 }
        XCTAssertTrue(spinUntil(10) { ok != nil })
        XCTAssertEqual(ok, true, snaps.notice?.text ?? "")
        let list = TemplateEntry.parse(templateList(userDir: templates.path))
        let mine = try XCTUnwrap(list.first { $0.name == "P1SW1 template" }, "\(list.map(\.name))")
        XCTAssertFalse(mine.builtin)
        XCTAssertEqual(mine.id, TemplateEntry.fileName("P1SW1 template"))
        XCTAssertNoThrow(try templateText(id: mine.id, userDir: templates.path))

        // A snapshot of a file works as of a device.
        _ = capture(snaps, "P1SW1 of a file")
    }

    func testP1SW1ImportPreviewIsOfferedThroughACore() {
        let (_, _, vm) = demoVM()
        XCTAssertNotNil(vm.transport.coreStateSource, "the import goes to the preview")
        let plain = DSPViewModel(transport: FakeTransport(autoSelect: false))
        XCTAssertNil(plain.transport.coreStateSource, "without a core the old options stay")
    }

    // MARK: - Words and parsing

    func testP1SW1CountdownWordsAreTheCores() {
        let words = CoreSnapshots.wordsFor(key: CoreSnapshots.countdownKey, fallback: "", needs: "{s}")
        XCTAssertFalse(words.isEmpty, "the core's notice table has guard.waiting")
        XCTAssertEqual(CoreSnapshots.countdownLine("in {s} s", remainingMs: 4001), "in 5 s")
        XCTAssertEqual(CoreSnapshots.countdownLine("in {s} s", remainingMs: 0), "in 0 s")
        XCTAssertFalse(CoreSnapshots.hubBackstop.isEmpty)
        XCTAssertEqual(CoreSnapshots.wordsFor(key: "p1sw1.no-such-key", fallback: "fallback"), "fallback")
    }

    func testP1SW1DiffAndBlocksParse() {
        let json = """
        {"ok":true,"rows":[{"block":"eq","section":"USB 1","item":"Band 1 gain","before":0,"after":2.5},
        {"block":"volumes","section":"Master","item":"Volume","before":-10,"after":-4},
        {"block":"names","section":"Output 1","item":"Name","before":"L","after":null}],"notes":["n"]}
        """
        let diff = SnapshotDiff.parse(json)
        XCTAssertTrue(diff.ok)
        XCTAssertEqual(diff.rows.count, 3)
        XCTAssertEqual(diff.rows[0].after, "2.5")
        XCTAssertEqual(diff.rows[1].before, "-10")
        XCTAssertEqual(diff.rows[2].after, "")
        XCTAssertEqual(diff.rows[0].place, "USB 1 · Band 1 gain")
        XCTAssertEqual(ImportBlock.defaults(diff), [.eq, .names])
        XCTAssertEqual(ImportBlock.options([.eq]), DspiConstants.IMPORT_BY_BLOCK | DspiConstants.IMPORT_EQ)
        XCTAssertFalse(SnapshotDiff.parse("{\"ok\":false,\"error\":\"bad\"}").ok)
        XCTAssertEqual(SnapshotDiff.parse("nope").error, "The changes could not be read.")
        XCTAssertEqual(TemplateEntry.fileName("a/b:c"), "a_b_c.\(PresetDocument.fileExtension)")
        XCTAssertEqual(OfflineDocumentInfo.parse("{\"offline\":false}"), nil)
        XCTAssertEqual(OfflineDocumentInfo.parse("{\"offline\":true,\"fileName\":\"x.dspipreset\",\"name\":\"X\"}")?.badge,
                       "File: x.dspipreset")
        XCTAssertEqual(CoreSnapshots.words(.InvalidArgument(msg: "not a snapshot — import it instead")),
                       "Not a snapshot — import it instead.")
    }

    #if os(macOS)
    func testP1SW1TheABKeyIsABareB() throws {
        func key(_ chars: String, _ mods: NSEvent.ModifierFlags = []) throws -> NSEvent {
            try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0,
                                           windowNumber: 0, context: nil, characters: chars,
                                           charactersIgnoringModifiers: chars, isARepeat: false, keyCode: 11))
        }
        XCTAssertTrue(CoreSnapshots.isBareB(try key("b")))
        XCTAssertFalse(CoreSnapshots.isBareB(try key("b", .command)), "Cmd-B is not the A/B key")
        XCTAssertFalse(CoreSnapshots.isBareB(try key("n")))
    }
    #endif
}
