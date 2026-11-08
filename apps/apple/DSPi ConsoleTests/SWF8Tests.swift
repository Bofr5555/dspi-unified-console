import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SWF8 (review c72): the prompt's settle never holds main for long.  Over
/// a hub session it returns at once (N1: the hub's echoes reach the core
/// through main, so waiting there cannot bring them in); elsewhere the whole
/// call, the hop behind work already on the core's queue included, is
/// bounded by `settleWindow` (N2).
final class SWF8Tests: XCTestCase {

    /// The longest a settle may hold main: its window, plus scheduling slack
    /// on a busy test machine.
    private static let mainBudget = CoreBackedTransport.settleWindow + 0.05

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    private func elapsed(_ body: () -> Void) -> TimeInterval {
        let start = Date()
        body()
        return Date().timeIntervalSince(start)
    }

    // MARK: - N1: a hub session

    /// The user drags a delay slider on the Mac's hub session and picks
    /// another preset at once.  The host answers every GET only through
    /// main, as the reviewer's worst case (a completion delivered on main
    /// behind a 15 s semaphore), and its echoes arrive on main.
    func testSWF8APromptRightAfterADragOnAHubDoesNotBlockMain() {
        let fake = FakeTransport(autoSelect: false)
        let host = MainCompletingTransport(fake)
        let t = CoreBackedTransport(link: .host(host, .hub))
        let vm = DSPViewModel(transport: t)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes over the hub")
        settle()
        XCTAssertNotNil(vm.coreMirror.attachedSource, "the view model follows the core")
        // From here a re-read the drain needs waits on main, as the
        // reviewer's case; the connect's reads above did not.
        host.completesOnMain = true

        let start = vm.outputDelayMS[0]
        let target = (start + 7).rounded()
        for ms in [start + 2.4, start + 4.8, start + 7.1] { vm.sendOutputDelayToDevice(output: 0, ms: ms.rounded()) }
        vm.setOutputDelay(output: 0, ms: target)

        // What the preset popup's selection does before it decides.
        XCTAssertLessThan(elapsed { vm.settleUnsavedChanges() }, 0.02, "the settle does not wait on a hub")
        var settled: [String]? = ["not called"]
        XCTAssertLessThan(elapsed { settled = t.settlePresetChanges() }, 0.02)
        XCTAssertNil(settled, "nil at once: the list as delivered decides")

        // The edit still reaches the core through the normal drain.
        XCTAssertTrue(spinUntil(5) { t.readState().outputDelayMs[0] == target }, "the drag reached the core")
    }

    // MARK: - N2: the hop behind queued core work

    /// A settle asked while the core's queue is busy (a preset load's lock
    /// wait, a fetchAll) gives up after `settleWindow`; the late job does
    /// nothing, and the drain delivers the edit anyway.
    func testSWF8TheSettleIsBoundedBehindQueuedCoreWork() {
        let t = CoreBackedTransport(link: .demo)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation })
        settle()

        let target = (vm.outputDelayMS[1] + 3).rounded()
        vm.sendOutputDelayToDevice(output: 1, ms: target)
        let busy = DispatchSemaphore(value: 0)
        t.coreQueue.async { _ = busy.wait(timeout: .now() + 2) }

        var settled: [String]? = ["not called"]
        let took = elapsed { settled = t.settlePresetChanges() }
        XCTAssertNil(settled, "the queue was busy past the window")
        XCTAssertGreaterThanOrEqual(took, CoreBackedTransport.settleWindow - 0.005, "it waited its window")
        XCTAssertLessThan(took, Self.mainBudget, "and no longer")
        XCTAssertLessThan(elapsed { vm.settleUnsavedChanges() }, Self.mainBudget, "nor through the view model")

        busy.signal()
        XCTAssertTrue(spinUntil(5) { t.readState().outputDelayMs[1] == target }, "the drain delivers the edit")
        // The queue is free again and the window is over: no wait at all.
        XCTAssertLessThan(elapsed { XCTAssertNil(t.settlePresetChanges()) }, 0.02)
    }

    /// With the queue free, the settle still settles inside the bound.
    func testSWF8AnUnblockedSettleStillSettlesInTime() {
        let t = CoreBackedTransport(link: .demo)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation })
        settle()

        let target = (vm.outputDelayMS[2] + 4).rounded()
        vm.sendOutputDelayToDevice(output: 2, ms: target)
        var settled: [String]?
        XCTAssertLessThan(elapsed { settled = t.settlePresetChanges() }, Self.mainBudget)
        XCTAssertNotNil(settled, "a SET just went out and the queue was free")
        XCTAssertEqual(t.readState().outputDelayMs[2], target, "drained before it returned")
    }

    // MARK: - c75-N3: the delay line is the core's

    private func demoVM(file: StaticString = #filePath, line: UInt = #line) -> (CoreBackedTransport, DSPViewModel) {
        let t = CoreBackedTransport(link: .demo)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes", file: file, line: line)
        settle()
        return (t, vm)
    }

    func testSWF8TheDelayLineIsTheCores() throws {
        let (t, vm) = demoVM()
        let line = try XCTUnwrap(t.readMaxDelayMs())
        XCTAssertEqual(vm.coreMaxDelayMS, line, "read with the fetch")
        XCTAssertEqual(vm.maxOutputDelayMS, line)
        XCTAssertEqual(DSPViewModel.delayLineMS(platform: vm.platformName), line, "the rule off the core agrees")
        XCTAssertNotEqual(line, 85, "not the Console's own maximum")
        XCTAssertEqual(DSPViewModel.delayLineMS(platform: "RP2350"), 42)
        XCTAssertEqual(DSPViewModel.delayLineMS(platform: "RP2040"), 21)
        XCTAssertEqual(DSPViewModel.delayLineMS(platform: ""), 21, "unknown: the shorter line")
    }

    /// A typed delay past the line: nothing is sent, the field's delay stays,
    /// and the status row says why in the core's words.
    func testSWF8ATypedDelayPastTheLineIsRefusedInTheCoresWords() {
        let (t, vm) = demoVM()
        let line = vm.maxOutputDelayMS
        vm.setOutputDelay(output: 0, ms: line)
        XCTAssertTrue(spinUntil(5) { t.readState().outputDelayMs[0] == line }, "the line itself is taken")
        settle()

        vm.setOutputDelay(output: 0, ms: line + 1)
        XCTAssertEqual(vm.outputDelayMS[0], line, "the delay kept")
        XCTAssertTrue(spinUntil(5) { DeviceLock.notice.message?.contains("out of range") == true },
                      "the refusal is posted")
        let words = DeviceLock.notice.message ?? ""
        XCTAssertTrue(words.contains("\(ValueField.delayText(line + 1)) ms"), words)
        XCTAssertTrue(words.contains("holds 0 to \(ValueField.delayText(line)) ms"), words)
        settle()
        XCTAssertEqual(t.readState().outputDelayMs[0], line, "nothing reached the device")

        vm.setOutputDelay(output: 0, ms: .nan)
        XCTAssertEqual(vm.outputDelayMS[0], line, "NaN is refused too")
        settle()
        XCTAssertEqual(t.readState().outputDelayMs[0], line)
    }

    /// A drag, a paste and a file's delay are held to the line.
    func testSWF8DragsAndPastesStopAtTheLine() {
        let (t, vm) = demoVM()
        let line = vm.maxOutputDelayMS
        vm.sendOutputDelayToDevice(output: 1, ms: 85)
        XCTAssertTrue(spinUntil(5) { t.readState().outputDelayMs[1] == line }, "the drag stops at the line")

        let out = 2
        vm.channelClipboard = DSPViewModel.ChannelClipboard(
            filters: [], crossover: nil, outputGainDB: nil, outputDelayMS: 85, outputMuted: nil, sourceName: "SWF8")
        vm.pasteChannelParams(eqChannel: vm.eqChannel(forOutput: out))
        XCTAssertEqual(vm.outputDelayMS[out], line, "the paste stops at the line")
        XCTAssertTrue(spinUntil(5) { t.readState().outputDelayMs[out] == line })
    }

    /// Off the core the platform's rule bounds it, worded the same way.
    func testSWF8OffTheCoreThePlatformRuleRefuses() {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == fake.generation })
        settle()
        XCTAssertNil(vm.coreMirror.attachedSource)
        let line = DSPViewModel.delayLineMS(platform: vm.platformName)
        XCTAssertEqual(vm.maxOutputDelayMS, line)
        let before = vm.outputDelayMS[0]
        vm.setOutputDelay(output: 0, ms: line + 5)
        XCTAssertEqual(vm.outputDelayMS[0], before)
        XCTAssertTrue(spinUntil(5) { DeviceLock.notice.message?.contains("holds 0 to \(ValueField.delayText(line)) ms") == true },
                      DeviceLock.notice.message ?? "no message")
    }
}

/// A hub host.  Its notifications arrive on main (the fake's fanout, as
/// `HubTransport`'s).  With `completesOnMain`, like a Link client delivering
/// its completions on main, it answers a GET from another thread only through
/// the main queue (bounded at 15 s, as `NetworkTransport`); off, it answers
/// on the caller's thread, as `HubTransport` does.
final class MainCompletingTransport: ForwardingTransport {
    private let flagLock = NSLock()
    private var _completesOnMain = false
    var completesOnMain: Bool {
        get { flagLock.lock(); defer { flagLock.unlock() }; return _completesOnMain }
        set { flagLock.lock(); _completesOnMain = newValue; flagLock.unlock() }
    }

    override func getControlResult(request: UInt8, value: UInt16, index: UInt16,
                                   length: UInt16) -> Result<Data, DSPi_Console.LinkStatus> {
        if !completesOnMain || Thread.isMainThread {
            return inner.getControlResult(request: request, value: value, index: index, length: length)
        }
        let done = DispatchSemaphore(value: 0)
        let lock = NSLock()
        var result: Result<Data, DSPi_Console.LinkStatus>?
        DispatchQueue.main.async {
            let r = self.inner.getControlResult(request: request, value: value, index: index, length: length)
            lock.lock(); result = r; lock.unlock()
            done.signal()
        }
        guard done.wait(timeout: .now() + 15) == .success else { return .failure(.timeout) }
        lock.lock(); defer { lock.unlock() }
        return result ?? .failure(.timeout)
    }
}
