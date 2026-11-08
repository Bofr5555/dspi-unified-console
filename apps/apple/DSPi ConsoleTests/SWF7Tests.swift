import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SWF7: the delay field while the slider drags shows the whole ms it
/// sends (SWF5-1); scroll steps from a fractional delay land on the whole-ms
/// grid (SWF5-2); an unsaved-changes prompt raised right after a raw SET
/// settles first, so the edit is in the core's list (c70-N1); a load refused
/// by the lock reads as the lock (c70-N2).
final class SWF7Tests: XCTestCase {

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

    /// Preset 3 holds the connect-time state and is the clean baseline.
    private func savedPreset3(_ vm: DSPViewModel, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(offMain { vm.savePreset(slot: 3) }, PRESET_OK, file: file, line: line)
        XCTAssertTrue(spinUntil(5) { vm.activePresetSlot == 3 && !vm.hasUnsavedChanges },
                      "the save is the baseline", file: file, line: line)
        settle()
    }

    // MARK: - SWF5-1, SWF5-2

    func testSWF7DelayFieldShowsTheWholeMsWhileDragging() {
        XCTAssertEqual(ValueField.delayShown(12.3457, dragging: true), 12, "what the drag sends")
        XCTAssertEqual(ValueField.delayShown(12.5, dragging: true), 13)
        XCTAssertEqual(ValueField.delayShown(0.4, dragging: true), 0)
        XCTAssertEqual(ValueField.delayShown(12.3457, dragging: false), 12.3457, "exact when not dragging")
        XCTAssertEqual(ValueField.delayShown(0.25, dragging: false), 0.25, "a fraction from a file or the device")
        XCTAssertEqual(ValueField.delayText(ValueField.delayShown(12.3457, dragging: true)), "12")
    }

    func testSWF7ScrollStepsLandOnTheWholeMsGrid() {
        let cases: [(Float, Float, Float)] = [
            // The panel field's Cmd-scroll (step 1).
            (0.25, 1, 1), (1.5, -1, 1), (1, 1, 2), (2, -1, 1), (0.5, -1, 0), (0.5, 1, 1), (12.75, 1, 13),
            // The Matrix field's scroll (5 ms per unit, any delta): from a
            // whole value as upstream, rounded; from a fraction to the grid
            // in the scroll direction, never against it.
            (3, 0.5, 4), (3, 0.2, 3), (3, -0.5, 3), (0.25, 0.1, 1), (0.25, -0.1, 0), (0.5, 5, 5),
            (12.75, -5, 8), (1.5, -0.5, 1), (0.5, 1, 1),
        ]
        for (from, delta, want) in cases {
            XCTAssertEqual(ValueField.wholeStep(from: from, by: delta), want, "\(from) by \(delta)")
        }
        XCTAssertEqual(ValueField.wholeStep(from: 0.25, by: 0), 0.25, "no scroll, no change")
    }

    // MARK: - c70-N1: the prompt right after a raw SET

    /// The user drags a delay slider and picks another preset at once.  The
    /// drag's SETs reach the core's list only with their echoes (one drain
    /// later); the prompt's settle drains them first, so the switch prompts
    /// with the drag in its list.
    func testSWF7APresetSwitchRightAfterADragPrompts() {
        let (t, vm) = demoVM()
        savedPreset3(vm)
        let start = vm.outputDelayMS[0]
        let target = (start + 7).rounded()

        // The drag: whole ms live (the slider sends `val.rounded()`), then
        // the release's commit.
        for ms in [start + 2.4, start + 4.8, start + 7.1] { vm.sendOutputDelayToDevice(output: 0, ms: ms.rounded()) }
        vm.setOutputDelay(output: 0, ms: target)
        // The main queue has not turned: no drain has reached the view model.
        XCTAssertFalse(vm.hasUnsavedChanges, "inside the echo window the list does not hold the drag yet")

        // What the preset popup's selection now does before it decides.
        vm.settleUnsavedChanges()
        XCTAssertTrue(vm.isDeviceConnected && vm.hasUnsavedChanges, "the switch prompts")
        let lines = vm.computeDiff().changes.map(\.description)
        XCTAssertTrue(lines.contains { $0.contains(" delay: ") && $0.hasSuffix("→ \(ValueField.delayText(target)) ms") },
                      "\(lines)")
        XCTAssertEqual(lines, t.readPresetChanges(), "the core's sentences")
    }

    /// The settle only acts inside the window: with no recent SET it leaves
    /// the list as delivered and does not wait.
    func testSWF7TheSettleActsOnlyRightAfterASet() {
        let (t, vm) = demoVM()
        savedPreset3(vm)
        XCTAssertNil(t.settlePresetChanges(), "no recent SET")

        vm.sendOutputDelayToDevice(output: 1, ms: (vm.outputDelayMS[1] + 3).rounded())
        let settled = t.settlePresetChanges()
        XCTAssertNotNil(settled, "a SET just went out")
        XCTAssertTrue(settled?.contains { $0.contains(" delay: ") } ?? false, "\(settled ?? [])")

        Thread.sleep(forTimeInterval: CoreBackedTransport.settleWindow + 0.05)
        let before = Date()
        XCTAssertNil(t.settlePresetChanges(), "the window is over")
        XCTAssertLessThan(Date().timeIntervalSince(before), 0.05, "and it did not wait")
        XCTAssertTrue(spinUntil(5) { vm.hasUnsavedChanges }, "the drain delivers it anyway")
    }

    /// A transport not on the core: the settle is a no-op, the Swift snapshot decides.
    func testSWF7TheSettleIsANoOpWithoutACore() {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == fake.generation })
        settle()
        vm.setOutputDelay(output: 0, ms: (vm.outputDelayMS[0] + 3).rounded())
        vm.settleUnsavedChanges()
        XCTAssertTrue(vm.hasUnsavedChanges)
        XCTAssertTrue(vm.corePresetChanges.isEmpty)
    }

    // MARK: - c70-N2

    func testSWF7ALockedLoadReadsAsTheLock() {
        XCTAssertEqual(presetLoadFailedText(PRESET_STATUS_LOCKED), "Failed to load preset (\(DeviceLock.lockedMessage)).")
        XCTAssertFalse(presetLoadFailedText(PRESET_STATUS_LOCKED).contains("254"))
        XCTAssertEqual(presetLoadFailedText(0xFF), "Failed to load preset (error 255).")
        XCTAssertEqual(presetLoadFailedText(PRESET_ERR_CRC), "Preset data is corrupted.")
    }
}
