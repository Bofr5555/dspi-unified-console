import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SWMAC1: Undo and Redo through the core's change journal
/// (Core/CoreUndo.swift), drags marked with `setDragging`, and the AutoEQ
/// requests' User-Agent.
final class CoreUndoSWMAC1Tests: XCTestCase {

    /// The core's demo device, the core behind it, and a view model on it.
    private func demoVM() -> (DspiCore, CoreBackedTransport, DSPViewModel) {
        let core = DspiCore()
        let t = CoreBackedTransport(link: .demo, core: core)
        let vm = DSPViewModel(transport: t)
        t.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        return (core, t, vm)
    }

    /// A typed write, as SW9's domains will send them, on the core's queue.
    private func setMasterVolume(_ t: CoreBackedTransport, _ core: DspiCore, _ db: Float) throws {
        try t.coreQueue.sync { try core.setMasterVolume(db: db) }
    }

    func testSWMAC1UndoAndRedoAMasterVolumeChange() throws {
        let (core, t, vm) = demoVM()
        XCTAssertEqual(vm.masterVolumeDB, -20)
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        try setMasterVolume(t, core, -9)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo }, "Undo is enabled after a typed write")
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -9 })

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -20 }, "undone: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { vm.undo.canRedo })
        XCTAssertFalse(vm.undo.canUndo)
        XCTAssertNil(vm.undo.notice, "a plain step says nothing")

        vm.undo.redo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -9 }, "redone: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo && !vm.undo.canRedo })
    }

    /// The domains' raw SETs are history only in the core: they leave Undo
    /// disabled (until SW9-SW14 move them to typed setters).
    func testSWMAC1RawWritesCannotBeUndone() {
        let (_, t, vm) = demoVM()
        t.sendControlRequest(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0,
                             data: withUnsafeBytes(of: Float(-7)) { Data($0) })
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -7 })
        XCTAssertFalse(t.readJournal().canUndo)
        XCTAssertFalse(vm.undo.canUndo)
    }

    /// A marked drag is one step however many writes it sends; unmarked,
    /// each write is its own.
    func testSWMAC1ADragIsOneUndoStep() throws {
        let (core, t, vm) = demoVM()
        t.setDragging(true)
        for db: Float in [-18, -15, -12] { try setMasterVolume(t, core, db) }
        t.setDragging(false)
        try setMasterVolume(t, core, -6)   // a typed value after the drag
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo && vm.masterVolumeDB == -6 })

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -12 }, "the typed value first: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == -20 }, "the whole drag in one step: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo })
    }

    func testSWMAC1AutoEQUserAgentIsTheCores() {
        let ua = AutoEQManager.session.configuration.httpAdditionalHeaders?["User-Agent"] as? String
        XCTAssertEqual(ua, "uConsole-for-DSPi/\(coreInfo().ffiVersion)")
        XCTAssertEqual(AutoEQManager.userAgent, ua)
        XCTAssertTrue(ua?.hasPrefix("uConsole-for-DSPi/") == true)
    }
}
