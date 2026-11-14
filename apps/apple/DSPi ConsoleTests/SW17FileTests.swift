import XCTest
import dspi_constants
@testable import DSPi_Console

/// WP SW17: the file actions on iOS write and read the Mac's files.  The
/// export and import code is shared (FileMenuActions, PresetDocumentFile), so
/// these run in both test bundles: on iOS they cover the document picker's
/// staging and the read-only refusal too.  Everything runs on the core's demo
/// device and the shared fixtures in tests/fixtures.
final class SW17FileTests: XCTestCase {

    // MARK: - Helpers

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    /// A view model on the core's demo device, after its connect-time read.
    private func demoVM(file: StaticString = #filePath, line: UInt = #line) -> DSPViewModel {
        let t = CoreBackedTransport(link: .demo)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes", file: file, line: line)
        settle()
        return vm
    }

    private static let fixtures = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // DSPi ConsoleTests
        .deletingLastPathComponent()   // apps/apple
        .deletingLastPathComponent()   // apps
        .deletingLastPathComponent()   // the repository
        .appendingPathComponent("tests/fixtures")

    private func fixture(_ name: String) throws -> Data {
        try Data(contentsOf: Self.fixtures.appendingPathComponent(name))
    }

    /// What the bytes go through on the way to the user's file: on iOS the
    /// document picker copies out the file staged in the app's sandbox.  On
    /// macOS the save panel's URL is written directly, so the bytes are the
    /// bytes.
    private func throughThePicker(_ data: Data, fileName: String) throws -> Data {
        #if os(iOS)
        let url = try DocumentPicker.stage(data, fileName: fileName)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }
        XCTAssertEqual(url.lastPathComponent, fileName)
        XCTAssertTrue(url.path.hasPrefix(FileManager.default.temporaryDirectory.path),
                      "staged inside the app's sandbox")
        return try Data(contentsOf: url)
        #else
        return data
        #endif
    }

    /// A band as the filter file can carry it (its own precision).
    private static func band(_ p: FilterParams) -> String {
        guard p.type != .flat else { return "flat" }
        var text = String(format: "%d %.1f Hz %.1f dB Q %.2f", p.type.rawValue, p.freq, p.gain, p.q)
        if p.type == .linkwitzTransform { text += String(format: " Qp %.2f", p.qp) }
        return text + (p.bypass ? " bypassed" : "")
    }

    private static func bands(_ list: [FilterParams], count: Int) -> [String] {
        (0..<count).map { $0 < list.count ? band(list[$0]) : "flat" }
    }

    /// The export without its "# Exported:" time stamp.
    private static func undated(_ text: String) -> String {
        text.components(separatedBy: "\n").filter { !$0.hasPrefix("# Exported:") }.joined(separator: "\n")
    }

    private func apply(_ doc: PresetDocument, to vm: DSPViewModel,
                       file: StaticString = #filePath, line: UInt = #line) -> PresetApplyReport? {
        var result: PresetApplyReport?
        // Volume levels stay off, as the import's options start: the demo
        // device keeps its master volume apart from presets (independent
        // mode), so the apply would skip it.
        let options = PresetApplyOptions(audioProcessing: true, volumeLevels: false, hardwareIO: true)
        PresetDocumentApply.apply(doc, to: vm, options: options) { result = $0 }
        XCTAssertTrue(spinUntil(60) { result != nil }, "the apply finishes", file: file, line: line)
        settle(0.6)
        return result
    }

    /// The JSON paths where what the device holds differs from every value
    /// the file carries.  Left out: `meta` (who saved it, when, from which
    /// build), the volume levels (not imported), and fields the file does not
    /// have (a newer field such as io.dacHwMute).  Delays compare exactly:
    /// the setters keep fractions since SWF5.
    private static func differences(device a: PresetDocument, file b: PresetDocument) throws -> [String] {
        func object(_ doc: PresetDocument) throws -> Any {
            var top = try JSONSerialization.jsonObject(with: PresetDocumentFile.encode(doc)) as? [String: Any] ?? [:]
            top["meta"] = nil
            if var global = top["global"] as? [String: Any] {
                global["masterVolumeDb"] = nil
                global["userVolumeDb"] = nil
                top["global"] = global
            }
            return top
        }
        var out: [String] = []
        func walk(_ x: Any?, _ y: Any?, _ path: String) {
            guard let y else { return }
            if let dx = x as? [String: Any], let dy = y as? [String: Any] {
                for key in Set(dx.keys).union(dy.keys).sorted() { walk(dx[key], dy[key], path + "." + key) }
            } else if let ax = x as? [Any], let ay = y as? [Any], ax.count == ay.count {
                for i in ax.indices { walk(ax[i], ay[i], path + "[\(i)]") }
            } else if let nx = x as? NSObject, let ny = y as? NSObject, nx == ny {
                return
            } else {
                out.append("\(path): \(x.map { "\($0)" } ?? "nil") vs \(y)")
            }
        }
        walk(try object(a), try object(b), "")
        return out
    }

    private func assertSameDocument(_ a: PresetDocument, _ b: PresetDocument, _ what: String,
                                    file: StaticString = #filePath, line: UInt = #line) throws {
        let diff = try Self.differences(device: a, file: b)
        XCTAssertTrue(diff.isEmpty, "\(what): \(diff.count) differences: \(diff.prefix(12).joined(separator: "; "))",
                      file: file, line: line)
    }

    // MARK: - Filter files

    /// Export Filters on one demo device, Import Filters (every channel the
    /// file offers) on another: the second device ends with the same bands,
    /// preamps, crossovers and output states, so its own export is the same
    /// file apart from the time stamp.
    func testSW17FilterExportImportRoundTripsOnTheDemoDevice() throws {
        let source = demoVM()
        source.setFilter(ch: 0, band: 0, p: FilterParams(type: .peaking, freq: 250, q: 1.2, gain: -3.5))
        source.setFilter(ch: 1, band: 2, p: FilterParams(type: .highShelf, freq: 8000, q: 0.71, gain: 2))
        source.setPreampChannel(channel: 0, db: -4.5)
        let out0 = source.eqChannel(forOutput: 0)
        source.setFilter(ch: out0, band: 1, p: FilterParams(type: .lowShelf, freq: 120, q: 0.8, gain: 4))
        settle(0.6)

        let text = FileMenuActions.generateExportString(vm: source)
        let bytes = try throughThePicker(Data(text.utf8), fileName: "DSPi Filters.txt")
        XCTAssertEqual(bytes, Data(text.utf8), "the file holds the export byte for byte")

        let target = demoVM()
        let contents = try XCTUnwrap(String(data: bytes, encoding: .utf8))
        let parsed = try XCTUnwrap(FileMenuActions.parseDSPiFile(contents, vm: target))
        let choices = FileMenuActions.channelChoices(for: parsed, vm: target)
        XCTAssertEqual(choices.count, target.numMatrixInputs + target.numOutputChannels, "every channel offered")
        XCTAssertTrue(choices.allSatisfy(\.selected))
        let report = FileMenuActions.applyDSPiFile(parsed, to: choices.map(\.id), vm: target)
        XCTAssertEqual(report.notes, "", "nothing skipped")
        settle(0.6)

        XCTAssertEqual(target.preampDB[0], -4.5, accuracy: 0.05)
        XCTAssertEqual(Self.band(try XCTUnwrap(target.channelData[0]?[0])), "\(FilterType.peaking.rawValue) 250.0 Hz -3.5 dB Q 1.20")
        XCTAssertEqual(Self.band(try XCTUnwrap(target.channelData[out0]?[1])), "\(FilterType.lowShelf.rawValue) 120.0 Hz 4.0 dB Q 0.80")
        XCTAssertEqual(Self.undated(FileMenuActions.generateExportString(vm: target)), Self.undated(text))
    }

    /// The shared filter fixtures (this app's current format, the Mac's
    /// legacy one, the Windows console's) import on the demo device: each
    /// channel they carry ends with their bands, and the device's own export
    /// carries them back.
    func testSW17FilterFixturesImportOnTheDemoDevice() throws {
        for name in ["MacOSFilters.txt", "DSPiFilters.txt", "WindowsFilters.txt"] {
            let vm = demoVM()
            let contents = try XCTUnwrap(String(data: try fixture(name), encoding: .utf8))
            XCTAssertTrue(contents.hasPrefix("# DSPi Console"), name)
            let parsed = try XCTUnwrap(FileMenuActions.parseDSPiFile(contents, vm: vm), name)
            let chosen = FileMenuActions.channelChoices(for: parsed, vm: vm).map(\.id)
            XCTAssertFalse(chosen.isEmpty, name)
            FileMenuActions.applyDSPiFile(parsed, to: chosen, vm: vm)
            settle(0.6)

            let again = try XCTUnwrap(FileMenuActions.parseDSPiFile(FileMenuActions.generateExportString(vm: vm), vm: vm))
            for ch in chosen {
                let count = vm.channelData[ch]?.count ?? 0
                let want = try XCTUnwrap(parsed.channels[ch])
                XCTAssertEqual(Self.bands(vm.channelData[ch] ?? [], count: count), Self.bands(want.filters, count: count),
                               "\(name): channel \(ch) on the device")
                XCTAssertEqual(Self.bands(again.channels[ch]?.filters ?? [], count: count),
                               Self.bands(want.filters, count: count), "\(name): channel \(ch) exported again")
                if let preamp = want.preamp, ch < vm.chOut1 {
                    XCTAssertEqual(vm.preampDB[ch], preamp, accuracy: 0.05, "\(name): preamp \(ch)")
                }
                if let xover = want.xover {
                    let n = DSPViewModel.crossoverBandsPerChannel
                    XCTAssertEqual(Self.bands(vm.xoverData[ch] ?? [], count: n), Self.bands(xover, count: n),
                                   "\(name): crossover \(ch)")
                }
            }
        }
    }

    /// A REW file goes on the channels picked (inputs ticked by default), its
    /// preamp on the inputs only.
    func testSW17REWFixtureImportsOnTheDemoDevice() throws {
        let vm = demoVM()
        let contents = try XCTUnwrap(String(data: try fixture("REWFilters.txt"), encoding: .utf8))
        XCTAssertFalse(contents.hasPrefix("# DSPi Console"))
        let parsed = FileMenuActions.parseREWFile(contents)
        XCTAssertFalse(parsed.filters.isEmpty)
        XCTAssertEqual(try XCTUnwrap(parsed.preamp), -6.5, accuracy: 0.01)

        let choices = FileMenuActions.channelChoices(for: parsed, vm: vm)
        XCTAssertEqual(choices.filter(\.selected).map(\.id), Array(0..<vm.numMatrixInputs), "inputs ticked")
        XCTAssertTrue(choices.contains { !$0.selected && $0.id >= vm.chOut1 }, "enabled outputs offered, unticked")
        XCTAssertTrue(FileMenuActions.importPrompt(for: parsed).contains("-6.5 dB preamp"))

        let out0 = vm.eqChannel(forOutput: 0)
        let report = FileMenuActions.applyREWFile(parsed, to: [0, out0], vm: vm)
        XCTAssertTrue(report.preampDropped, "an output has no preamp")
        settle(0.6)
        XCTAssertEqual(vm.preampDB[0], -6.5, accuracy: 0.05)
        for ch in [0, out0] {
            let count = vm.channelData[ch]?.count ?? 0
            XCTAssertEqual(Self.bands(vm.channelData[ch] ?? [], count: count), Self.bands(parsed.filters, count: count),
                           "channel \(ch)")
        }
    }

    // MARK: - Device configuration (.dspipreset)

    /// The Mac's reference file decodes and encodes back to the same bytes,
    /// and the export picker hands those bytes on unchanged.
    func testSW17MacPresetFileIsWrittenBackByteForByte() throws {
        let original = try fixture("preset/mac-swift-v32.dspipreset")
        let encoded = try PresetDocumentFile.encode(PresetDocumentFile.decode(original))
        XCTAssertEqual(String(decoding: encoded, as: UTF8.self), String(decoding: original, as: UTF8.self))
        let name = "\(FileMenuActions.configurationName).\(PresetDocument.fileExtension)"
        XCTAssertEqual(try throughThePicker(encoded, fileName: name), encoded)
        XCTAssertTrue(FileMenuActions.configurationTypes.contains { $0.preferredFilenameExtension == "dspipreset" })
    }

    /// The Mac's and the Qt console's reference files apply on the demo
    /// device: what the device then holds, captured again, is the file.
    func testSW17PresetFixturesApplyOnTheDemoDevice() throws {
        for name in ["preset/mac-swift-v32.dspipreset", "preset/qt-core-v32.dspipreset"] {
            let doc = try PresetDocumentFile.decode(try fixture(name))
            let vm = demoVM()
            XCTAssertTrue(FileMenuActions.configurationImportPrompt(doc, vm: vm).hasPrefix("Saved from RP2350"), name)
            let report = try XCTUnwrap(apply(doc, to: vm), name)
            XCTAssertFalse(report.lockedOut, name)
            XCTAssertTrue(report.missingChannels.isEmpty, "\(name): \(report.missingChannels)")
            XCTAssertTrue(report.skipped.isEmpty, "\(name): \(report.skipped)")
            try assertSameDocument(PresetDocument.capture(from: vm), doc, name)
        }
    }

    /// Export Device Configuration on one demo device, Import on another
    /// (every option on): the second device captures the same document.
    func testSW17PresetExportImportRoundTripsOnTheDemoDevice() throws {
        let source = demoVM()
        source.setFilter(ch: 0, band: 3, p: FilterParams(type: .peaking, freq: 3150, q: 2, gain: 5))
        source.setPreampChannel(channel: 1, db: -2)
        settle(0.6)
        XCTAssertNil(FileMenuActions.configurationExportRefusal(source))
        let doc = PresetDocument.capture(from: source, name: FileMenuActions.configurationName)
        let bytes = try throughThePicker(PresetDocumentFile.encode(doc),
                                         fileName: "\(FileMenuActions.configurationName).dspipreset")
        XCTAssertEqual(bytes, try PresetDocumentFile.encode(doc))

        let target = demoVM()
        let report = try XCTUnwrap(apply(try PresetDocumentFile.decode(bytes), to: target))
        XCTAssertTrue(report.isClean, "\(report.missingChannels) \(report.skipped)")
        XCTAssertTrue(FileMenuActions.configurationImportResultText(report)
            .hasSuffix("Save them to a preset slot to keep them."))
        try assertSameDocument(PresetDocument.capture(from: target), doc, "round trip")
    }

    /// No device: nothing to export.
    func testSW17ConfigurationExportNeedsADevice() {
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        XCTAssertEqual(FileMenuActions.configurationExportRefusal(vm)?.key, "disconnected")   // -- WP P1SW3 -- the core's row
    }

    // MARK: - Read-only connections (iOS)

    #if os(iOS)
    /// On a read-only connection (older firmware, SW8) both imports are
    /// refused before the picker opens, in the core's words (SWF4); exports
    /// stay available.  On current firmware nothing is refused.
    func testSW17ImportIsRefusedOnAReadOnlyConnection() throws {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                       length: BULK_PARAMS_SIZE))
        let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: CoreMirrorSW8Tests.v17Image(v32)), .fake))
        let old = DSPViewModel(transport: core)
        XCTAssertTrue(spinUntil(30) { old.isDeviceConnected && old.fullReadGeneration == core.generation })
        settle()
        XCTAssertTrue(old.deviceReadOnly)
        XCTAssertEqual(FileMenuActions.importRefusal(old, needsDevice: true)?.key, "read_only")   // -- WP P1SW3 --
        XCTAssertEqual(FileMenuActions.importRefusal(old, needsDevice: false)?.key, "read_only")
        XCTAssertNil(FileMenuActions.configurationExportRefusal(old), "exporting reads only")

        let current = demoVM()
        XCTAssertNil(FileMenuActions.importRefusal(current, needsDevice: true))
        let none = DSPViewModel(transport: FakeTransport(autoSelect: false))
        XCTAssertEqual(FileMenuActions.importRefusal(none, needsDevice: true)?.key, "disconnected")
        XCTAssertNil(FileMenuActions.importRefusal(none, needsDevice: false), "filters import into the view")
    }

    /// The import's sheets are presented over the app and go away again; a
    /// progress sheet ended while it is still animating in (a quick apply)
    /// still closes, and what follows it runs once it has gone.
    func testSW17ImportSheetsPresentAndClose() throws {
        let root = try XCTUnwrap(UIApplication.shared.topViewController, "the test host shows the app")
        XCTAssertTrue(spinUntil(10) { root.presentedViewController == nil })

        FileImportSheets.chooseChannels(message: "m", choices: [.init(id: 0, name: "USB 1", selected: true)]) { _ in }
        XCTAssertTrue(spinUntil(5) { root.presentedViewController != nil }, "the channel sheet is up")
        let sheet = try XCTUnwrap(root.presentedViewController)
        if UIDevice.current.userInterfaceIdiom == .phone {
            XCTAssertEqual(sheet.sheetPresentationController?.detents.count, 2, "half height first on a phone")
        }
        let gone = expectation(description: "dismissed")
        sheet.dismiss(animated: false) { gone.fulfill() }
        wait(for: [gone], timeout: 5)

        let progress = FileImportSheets.ProgressSheet()
        progress.begin()
        progress.update(0.5)
        var followed = 0
        progress.end { followed += 1 }
        XCTAssertTrue(spinUntil(5) { followed == 1 }, "the follow-up runs")
        XCTAssertNil(root.presentedViewController, "and the progress sheet has gone first")
        settle()
        XCTAssertEqual(followed, 1, "once")
    }
    #endif
}
