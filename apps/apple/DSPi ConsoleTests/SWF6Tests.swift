import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SWF6: through the core, the unsaved-changes flag and the prompt's list
/// are the core's (`presetChanges`, CG7), so an edit made right after a preset
/// load stays unsaved (CG33); the transports not on the core keep the Swift
/// snapshot.  Every test builds its own transport, whatever the launch flag.
final class SWF6Tests: XCTestCase {

    // MARK: - Helpers

    /// The core's demo device and a view model on it, after its connect-time read.
    private func demoVM(file: StaticString = #filePath, line: UInt = #line) -> (CoreBackedTransport, DSPViewModel) {
        let t = CoreBackedTransport(link: .demo)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes", file: file, line: line)
        settle()
        return (t, vm)
    }

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    /// Run `body` off main while the main run loop keeps turning.
    private func offMain<T>(_ body: @escaping () -> T) -> T {
        var out: T?
        DispatchQueue.global(qos: .userInitiated).async { let v = body(); DispatchQueue.main.async { out = v } }
        XCTAssertTrue(spinUntil(20) { out != nil }, "the call returns")
        return out!
    }

    /// One GET behind every earlier SET on the transport's queue.
    private func flush(_ t: any DeviceTransport) {
        _ = t.getControlResult(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7)
    }

    private static func db(_ v: Float) -> String { String(format: "%.1f dB", v) }

    /// Records the state changes the core publishes.
    private final class Changes {
        var seen: [CoreStateChange] = []
        var token: AnyCancellable?
        init(_ t: CoreBackedTransport) { token = t.addStateObserver { [weak self] in self?.seen.append($0) } }
        /// The re-read a preset load notified.
        var presetRefresh: CoreStateChange? { seen.first { $0.presetLoaded && $0.refreshed } }
    }

    /// Preset 3 holds the connect-time state and is the clean baseline; the
    /// preamp of input 0 then and a different value to edit it to.
    private func savedPreset3(_ t: CoreBackedTransport, _ vm: DSPViewModel,
                              file: StaticString = #filePath, line: UInt = #line) -> (Float, Float) {
        XCTAssertEqual(offMain { vm.savePreset(slot: 3, confirmed: .savePreset()) }, PRESET_OK, file: file, line: line)
        XCTAssertTrue(spinUntil(5) { vm.activePresetSlot == 3 && !vm.hasUnsavedChanges },
                      "the save is the baseline", file: file, line: line)
        let saved = vm.preampDB[0]
        return (saved, ((saved + 3) * 10).rounded() / 10)
    }

    // MARK: - The load race (CG33 through the Swift app)

    /// This console loads a preset, and the user edits before the device's
    /// PRESET_LOADED / BULK_INVALIDATED(PRESET) are processed.  The core's
    /// re-read on those carries the edit; the old Swift rule took its snapshot
    /// at that re-read and so called the edit saved.  Now it stays unsaved,
    /// against the preset's own value, in the prompt's list too.
    func testSWF6AnEditRightAfterOwnLoadStaysUnsaved() {
        let (t, vm) = demoVM()
        let (saved, edited) = savedPreset3(t, vm)
        let changes = Changes(t)

        t.holdDrainForTest(true)   // the load's notifications wait: the window is open
        XCTAssertEqual(offMain { vm.loadPreset(slot: 3) }, PRESET_OK)
        settle()
        XCTAssertFalse(vm.hasUnsavedChanges, "the loaded preset is the baseline")

        vm.setPreampChannel(channel: 0, db: edited)   // right after the load
        flush(t)
        settle()                                      // its echo is queued behind the load's notifications
        t.holdDrainForTest(false)

        XCTAssertTrue(spinUntil(10) { changes.presetRefresh != nil }, "the load's notifications are processed")
        let refresh = changes.presetRefresh
        XCTAssertEqual(refresh?.state.inputPreampDb.first, edited,
                       "the load's re-read already holds the edit (where the Swift snapshot was taken)")
        settle()
        XCTAssertTrue(vm.hasUnsavedChanges, "the edit made after the load stays unsaved")
        let summary = vm.computeDiff().summary
        XCTAssertTrue(summary.contains("• Preamp L: \(Self.db(saved)) → \(Self.db(edited))"), summary)
        XCTAssertTrue(spinUntil(5) { vm.computeDiff().changes.map(\.description) == t.readPresetChanges() },
                      "the core's sentences, as they are")
    }

    /// The same window after a load another host made (a raw load the view
    /// model does not know of): the edit stays unsaved, from the value saved
    /// before, since the preset's own value under it cannot be read.
    func testSWF6AnEditRightAfterAnotherHostsLoadStaysUnsaved() {
        let (t, vm) = demoVM()
        let (saved, edited) = savedPreset3(t, vm)
        let changes = Changes(t)

        t.holdDrainForTest(true)
        XCTAssertEqual(offMain { t.getControlRequest(request: REQ_PRESET_LOAD, value: 3, index: 2, length: 1) },
                       Data([PRESET_OK]))
        vm.setPreampChannel(channel: 0, db: edited)
        flush(t)
        settle()
        t.holdDrainForTest(false)

        XCTAssertTrue(spinUntil(10) { changes.presetRefresh != nil }, "the load's notifications are processed")
        settle()
        XCTAssertTrue(vm.hasUnsavedChanges, "the edit made after the load stays unsaved")
        XCTAssertTrue(vm.computeDiff().summary.contains("Preamp L: \(Self.db(saved)) → \(Self.db(edited))"),
                      vm.computeDiff().summary)
    }

    /// No edit in the window: the loaded preset is clean, and an edit from
    /// before the load is gone with it.
    func testSWF6ALoadWithNoEditInTheWindowIsClean() {
        let (t, vm) = demoVM()
        let (_, edited) = savedPreset3(t, vm)
        let changes = Changes(t)
        vm.setPreampChannel(channel: 0, db: edited)
        XCTAssertTrue(spinUntil(5) { vm.hasUnsavedChanges }, "an edit against the saved preset")

        XCTAssertEqual(offMain { vm.loadPreset(slot: 3) }, PRESET_OK)
        XCTAssertTrue(spinUntil(10) { changes.presetRefresh != nil })
        settle()
        XCTAssertFalse(vm.hasUnsavedChanges, vm.computeDiff().summary)
        XCTAssertTrue(vm.computeDiff().changes.isEmpty)
    }

    // MARK: - Which path decides

    /// Through the core the flag and the list are the core's, delivered with
    /// each change, and a save makes the core's baseline.  The prompt keeps the
    /// Mac's layout: one "• " line per sentence.
    func testSWF6TheCoreDecidesWhenTheDeviceIsOnTheCore() {
        let (t, vm) = demoVM()
        let (saved, edited) = savedPreset3(t, vm)
        XCTAssertTrue(vm.unsavedChangesFromCore)

        vm.setPreampChannel(channel: 0, db: edited)
        XCTAssertTrue(spinUntil(5) { vm.hasUnsavedChanges })
        // A linked input pair writes twice: wait for both echoes.
        XCTAssertTrue(spinUntil(5) { vm.corePresetChanges == t.readPresetChanges() }, "the core's list, delivered")
        let core = t.readPresetChanges()
        XCTAssertFalse(core.isEmpty)
        XCTAssertEqual(vm.computeDiff().summary, core.map { "• \($0)" }.joined(separator: "\n"))
        XCTAssertTrue(core.contains("Preamp L: \(Self.db(saved)) → \(Self.db(edited))"), "\(core)")

        XCTAssertEqual(offMain { vm.savePreset(slot: 3, confirmed: .savePreset()) }, PRESET_OK)
        XCTAssertTrue(spinUntil(5) { !vm.hasUnsavedChanges }, "the save is the core's baseline")
        XCTAssertTrue(t.readPresetChanges().isEmpty)

        // A disconnect forgets the list with the connection.
        t.disconnect()
        XCTAssertTrue(spinUntil(5) { !vm.isDeviceConnected })
        XCTAssertFalse(vm.unsavedChangesFromCore)
        XCTAssertTrue(vm.corePresetChanges.isEmpty)
    }

    /// A transport not on the core (the Swift fake, as USB without
    /// `-CoreTransport`, MIDI and the Link client): the Swift snapshot decides,
    /// and its list names the section.
    func testSWF6WithoutACoreTheSwiftSnapshotDecides() {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == fake.generation })
        settle()
        XCTAssertFalse(vm.unsavedChangesFromCore)
        XCTAssertNotNil(vm.savedSnapshot, "the connect-time read took the Swift baseline")
        XCTAssertFalse(vm.hasUnsavedChanges)

        vm.setPreampChannel(channel: 0, db: ((vm.preampDB[0] + 3) * 10).rounded() / 10)
        XCTAssertTrue(vm.hasUnsavedChanges, "at once, from the view model's own values")
        XCTAssertFalse(vm.computeDiff().changes.first?.category.isEmpty ?? true)
        XCTAssertTrue(vm.corePresetChanges.isEmpty)
    }
}
