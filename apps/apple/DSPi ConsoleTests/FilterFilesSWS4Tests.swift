import XCTest
import dspi_core
@testable import DSPi_Console

/// WP SWS4: File > Import Filters and Export Filters through the core
/// (`filtersInspect`, `filtersImport`, `filtersExport`), with the picker's
/// offer, the notices and the opt-out's Swift path kept apart.  Every file
/// format the app reads, on the core's demo device, from tests/fixtures.
final class FilterFilesSWS4Tests: XCTestCase, CoreDomainTesting {

    // MARK: - Helpers

    private static let fixtures = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // DSPi ConsoleTests
        .deletingLastPathComponent()   // apps/apple
        .deletingLastPathComponent()   // apps
        .deletingLastPathComponent()   // the repository
        .appendingPathComponent("tests/fixtures")

    /// The formats: this app's (macOS) file, the older Console file, the
    /// Windows Console's, and a REW list.
    private static let formats = ["MacOSFilters.txt", "DSPiFilters.txt", "WindowsFilters.txt", "REWFilters.txt"]

    private func fixture(_ name: String) throws -> String {
        try String(contentsOf: Self.fixtures.appendingPathComponent(name), encoding: .utf8)
    }

    /// The export without its "# Exported:" time stamp (the one line the two
    /// writers spell differently: the core writes Unix time, the opt-out a
    /// local date).
    private static func undated(_ text: String) -> String {
        text.components(separatedBy: "\n").filter { !$0.hasPrefix("# Exported:") }.joined(separator: "\n")
    }

    private func step(_ text: String, _ vm: DSPViewModel,
                      file: StaticString = #filePath, line: UInt = #line) -> FileMenuActions.FilterImportStep? {
        var out: FileMenuActions.FilterImportStep?
        FileMenuActions.filterImportStep(text, vm: vm) { out = $0 }
        XCTAssertTrue(spinUntil(10) { out != nil }, "the file is read", file: file, line: line)
        return out
    }

    private func offer(_ text: String, _ vm: DSPViewModel,
                       file: StaticString = #filePath, line: UInt = #line) throws -> FileMenuActions.FilterImportOffer {
        switch step(text, vm, file: file, line: line) {
        case .offer(let offer)?: return offer
        case .refused(let notice)?: XCTFail("refused: \(notice.line)", file: file, line: line)
        case nil: break
        }
        throw XCTSkip("no offer")
    }

    /// Import `text` on every channel the picker offers; the notice.
    private func importAll(_ text: String, _ vm: DSPViewModel,
                           file: StaticString = #filePath, line: UInt = #line) throws -> ConsoleNotice {
        let offer = try offer(text, vm, file: file, line: line)
        var notice: ConsoleNotice?
        offer.apply(offer.choices.map(\.id)) { notice = $0 }
        XCTAssertTrue(spinUntil(30) { notice != nil }, "the import finishes", file: file, line: line)
        settle(0.6)
        return try XCTUnwrap(notice, file: file, line: line)
    }

    private func export(_ vm: DSPViewModel, file: StaticString = #filePath, line: UInt = #line) throws -> String {
        var outcome: FileMenuActions.FilterExportOutcome?
        FileMenuActions.filterExportText(vm: vm) { outcome = $0 }
        XCTAssertTrue(spinUntil(10) { outcome != nil }, "the export finishes", file: file, line: line)
        switch outcome {
        case .text(let text)?: return text
        case .refused(let notice)?: XCTFail("export refused: \(notice.line)", file: file, line: line)
        case nil: break
        }
        return ""
    }

    /// The first band each fixture gives the first input.
    private static let firstBand: [String: (FilterType, Float, Float)] = [
        "MacOSFilters.txt": (.lowShelf, 105, 8.8),
        "DSPiFilters.txt": (.lowShelf, 105, 8.8),
        "WindowsFilters.txt": (.peaking, 105, 8.8),
        "REWFilters.txt": (.peaking, 31.5, -4.2),
    ]

    // MARK: - Round trip, per format

    /// Each format read by the core lands on the device, and the core's
    /// export of it reads back (on a second device) to the same file.
    func testSWS4EachFormatRoundTripsThroughTheCore() throws {
        for name in Self.formats {
            let (_, _, a) = demoVM()
            let notice = try importAll(try fixture(name), a)
            XCTAssertTrue(notice.title.hasPrefix("Filters imported to"), "\(name): \(notice.line)")
            XCTAssertEqual(notice.kind, .info, name)

            let (type, freq, gain) = try XCTUnwrap(Self.firstBand[name])
            XCTAssertTrue(spinUntil(5) { a.channelData[0]?.first?.type == type }, "\(name): the band landed")
            let band = try XCTUnwrap(a.channelData[0]?.first)
            XCTAssertEqual(band.freq, freq, accuracy: 0.05, name)
            XCTAssertEqual(band.gain, gain, accuracy: 0.05, name)

            let first = try export(a)
            XCTAssertTrue(first.hasPrefix("# DSPi Console Filter Settings\n"), name)
            XCTAssertTrue(first.contains("# Format: 2\n"), name)

            let (_, _, b) = demoVM()
            _ = try importAll(first, b)
            let second = try export(b)
            XCTAssertEqual(Self.undated(second), Self.undated(first), "\(name): the export reads back to itself")
        }
    }

    /// The picker's offer is the core's: a Console file offers the channels
    /// it has, ticked; a REW list the inputs ticked and the outputs not.
    func testSWS4ThePickerOffersTheCoresChannels() throws {
        let (_, _, vm) = demoVM()
        let native = try offer(try fixture("MacOSFilters.txt"), vm)
        XCTAssertEqual(native.prompt, FileMenuActions.filterImportPrompt(native: true, filters: 0, preamp: nil))
        XCTAssertEqual(native.choices.map(\.id), [0, 1, vm.eqChannel(forOutput: 0), vm.eqChannel(forOutput: 8)])
        XCTAssertTrue(native.choices.allSatisfy(\.selected))

        let rew = try offer(try fixture("REWFilters.txt"), vm)
        XCTAssertTrue(rew.prompt.hasPrefix("Found "), rew.prompt)
        XCTAssertTrue(rew.prompt.contains("-6.5 dB preamp"), rew.prompt)
        let inputs = rew.choices.filter { $0.id < vm.chOut1 }
        XCTAssertEqual(inputs.count, vm.numMatrixInputs)
        XCTAssertTrue(inputs.allSatisfy(\.selected))
        XCTAssertTrue(rew.choices.filter { $0.id >= vm.chOut1 }.allSatisfy { !$0.selected })

        guard case .refused(let notice)? = step("not a filter file at all", vm) else {
            return XCTFail("a file with nothing in it is refused")
        }
        XCTAssertEqual(notice.key, "io")
        XCTAssertEqual(notice.detail, "No valid filters found in the file.")
    }

    // MARK: - One undo step

    /// A whole import (bands, preamps, crossovers on several channels) is
    /// one undo step: one Undo gives back the file before it, Redo the import.
    func testSWS4AnImportIsOneUndoStep() throws {
        let (_, _, vm) = demoVM()
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        let before = Self.undated(try export(vm))
        _ = try importAll(try fixture("MacOSFilters.txt"), vm)
        let after = Self.undated(try export(vm))
        XCTAssertNotEqual(after, before, "the import changed the device")

        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo }, "Undo is enabled after the import")
        vm.undo.undo()
        XCTAssertTrue(spinUntil(10) { !vm.undo.canUndo && vm.undo.canRedo }, "one Undo, and it was the only step")
        settle(0.6)
        XCTAssertEqual(Self.undated(try export(vm)), before, "one Undo undoes it all")
        vm.undo.redo()
        XCTAssertTrue(spinUntil(10) { vm.undo.canUndo && !vm.undo.canRedo }, "one Redo")
        settle(0.6)
        XCTAssertEqual(Self.undated(try export(vm)), after, "one Redo redoes it all")
    }

    // MARK: - A locked import

    /// Another console holds the device lock: the import writes nothing,
    /// leaves nothing to undo, and says so in the core's words.
    func testSWS4ALockedImportWritesNothing() throws {
        let fake = FakeTransport(autoSelect: false)
        let host = RefusingLock(fake)
        let core = DspiCore()
        let t = CoreBackedTransport(link: .host(host, .fake), core: core)
        let vm = DSPViewModel(transport: t)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        settle()

        let text = try fixture("MacOSFilters.txt")
        let before = Self.undated(try export(vm))
        let offer = try offer(text, vm)   // reading the file takes no lock
        host.refuse = true

        var notice: ConsoleNotice?
        offer.apply(offer.choices.map(\.id)) { notice = $0 }
        XCTAssertTrue(spinUntil(30) { notice != nil }, "the import finishes")
        settle(0.6)

        XCTAssertGreaterThan(host.refusals, 0, "the core asked for the lock")
        let sent = host.setsWhileRefused.map { String(format: "0x%02X", $0) }
        XCTAssertEqual(sent, [], "no SET went out")
        XCTAssertEqual(Self.undated(try export(vm)), before, "nothing changed")
        XCTAssertFalse(vm.undo.canUndo, "nothing to undo")

        let refusal = t.coreQueue.sync { core.filtersImport(text: text, wires: Data([0])) }
        let error = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: Data(refusal.utf8)) as? [String: Any])?["error"] as? String)
        XCTAssertFalse(error.isEmpty)
        XCTAssertEqual(notice?.title, error, "the core's sentence")
        XCTAssertEqual(notice?.kind, .warning)
    }

    // MARK: - The export, byte for byte

    /// The core's export is the old Swift writer's, byte for byte, for every
    /// fixture and for the demo device as it starts.  The one intended
    /// difference is the "# Exported:" stamp's spelling (Unix time from the
    /// core, a local date from Swift), which both write as the second line.
    func testSWS4TheCoresExportIsTheSwiftExport() throws {
        let (_, _, fresh) = demoVM()
        try assertSameExport(fresh, "the demo device")
        for name in Self.formats {
            let (_, _, vm) = demoVM()
            _ = try importAll(try fixture(name), vm)
            try assertSameExport(vm, name)
        }
    }

    private func assertSameExport(_ vm: DSPViewModel, _ what: String,
                                  file: StaticString = #filePath, line: UInt = #line) throws {
        let core = try export(vm, file: file, line: line)
        let swift = FileMenuActions.generateExportString(vm: vm)
        XCTAssertEqual(core.components(separatedBy: "\n")[safe: 1]?.hasPrefix("# Exported: "), true, what,
                       file: file, line: line)
        XCTAssertEqual(swift.components(separatedBy: "\n")[safe: 1]?.hasPrefix("# Exported: "), true, what,
                       file: file, line: line)
        let (a, b) = (Self.undated(core), Self.undated(swift))
        if a != b {
            let la = a.components(separatedBy: "\n"), lb = b.components(separatedBy: "\n")
            let first = zip(la, lb).enumerated().first { $0.element.0 != $0.element.1 }
            XCTFail("\(what): the exports differ at line \(first?.offset ?? min(la.count, lb.count)): "
                    + "core \(first?.element.0 ?? "<end>") / swift \(first?.element.1 ?? "<end>")",
                    file: file, line: line)
        }
    }

    // MARK: - The opt-out

    /// Without a core (`-CoreTransport NO`) the opt-out's Swift reader and
    /// writer still serve both menu items.
    func testSWS4TheOptOutKeepsItsSwiftPath() throws {
        let vm = DSPViewModel(transport: FakeTransport())
        XCTAssertTrue(spinUntil(10) { vm.isDeviceConnected })
        settle()
        XCTAssertNil(vm.transport.coreStateSource)

        let text = try export(vm)
        XCTAssertEqual(Self.undated(text), Self.undated(FileMenuActions.generateExportString(vm: vm)))

        let offer = try offer(try fixture("MacOSFilters.txt"), vm)
        var notice: ConsoleNotice?
        offer.apply(offer.choices.map(\.id)) { notice = $0 }
        XCTAssertTrue(spinUntil(5) { notice != nil })
        XCTAssertTrue(notice?.title.hasPrefix("Filters imported") == true, notice?.line ?? "")
        XCTAssertEqual(vm.channelData[0]?.first?.type, .lowShelf)

        guard case .refused(let refused)? = step("not a filter file at all", vm) else {
            return XCTFail("refused")
        }
        XCTAssertEqual(refused.detail, "No valid filters found in the file.")
    }
}

/// WP SWS4: a host transport whose device lock another console holds while
/// `refuse` is set; it records every SET sent meanwhile.
private final class RefusingLock: ForwardingTransport {
    private let stateLock = NSLock()
    private var _refuse = false
    private var _refusals = 0
    private var _sets: [UInt8] = []

    var refuse: Bool {
        get { stateLock.lock(); defer { stateLock.unlock() }; return _refuse }
        set { stateLock.lock(); _refuse = newValue; stateLock.unlock() }
    }
    var refusals: Int { stateLock.lock(); defer { stateLock.unlock() }; return _refusals }
    var setsWhileRefused: [UInt8] { stateLock.lock(); defer { stateLock.unlock() }; return _sets }

    override func acquireDeviceLock(reason: String, timeout: TimeInterval) -> Bool {
        stateLock.lock()
        let refused = _refuse
        if refused { _refusals += 1 }
        stateLock.unlock()
        return refused ? false : super.acquireDeviceLock(reason: reason, timeout: timeout)
    }

    override func sendControlRequest(request: UInt8, value: UInt16, index: UInt16, data: Data) {
        stateLock.lock()
        if _refuse { _sets.append(request) }
        stateLock.unlock()
        super.sendControlRequest(request: request, value: value, index: index, data: data)
    }
}
