import XCTest
import dspi_core
@testable import DSPi_Console

/// WP SW10: the harness of the domain moves' tests (CoreDomainSW9Tests,
/// CoreDomainSW10Tests), taken out of SW9's: a view model on the core's demo
/// device or on Swift's FakeTransport under the core, the core's queue
/// drained, its journal read, and Undo and Redo as the Edit menu does them.
protocol CoreDomainTesting: XCTestCase {}

extension CoreDomainTesting {

    func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    /// The core's demo device, the core behind it, and a view model on it.
    func demoVM() -> (DspiCore, CoreBackedTransport, DSPViewModel) {
        let core = DspiCore()
        let t = CoreBackedTransport(link: .demo, core: core)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        settle()
        return (core, t, vm)
    }

    /// Swift's FakeTransport under the core (CG26): the device keeps its
    /// state across the core's reconnects, and every transfer is the core's.
    func hostVM() -> (FakeTransport, DspiCore, CoreBackedTransport, DSPViewModel) {
        let fake = FakeTransport(autoSelect: false)
        let core = DspiCore()
        let t = CoreBackedTransport(link: .host(fake, .fake), core: core)
        let vm = DSPViewModel(transport: t)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        settle()
        return (fake, core, t, vm)
    }

    /// Every write queued so far has run on the core's queue.
    func flush(_ t: CoreBackedTransport) {
        t.coreQueue.sync {}
    }

    /// The journal entries this connection's own edits made for `path`.
    func entries(_ core: DspiCore, _ t: CoreBackedTransport, _ path: String) -> [HistoryEntry] {
        flush(t)
        return t.coreQueue.sync { core.history(n: 200) }.filter { $0.path == path && $0.source == .local }
    }

    /// Undo, then Redo, as the Edit menu does, and what is shown after each.
    func undoAndRedo(_ vm: DSPViewModel, file: StaticString = #filePath, line: UInt = #line,
                     undone: () -> Bool, redone: () -> Bool) {
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo }, "Undo is enabled after the edit", file: file, line: line)
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5, undone), "undone", file: file, line: line)
        XCTAssertTrue(spinUntil(5) { vm.undo.canRedo }, "Redo is enabled", file: file, line: line)
        vm.undo.redo()
        XCTAssertTrue(spinUntil(5, redone), "redone", file: file, line: line)
        XCTAssertNil(vm.undo.notice, "a plain step says nothing", file: file, line: line)
    }
}
