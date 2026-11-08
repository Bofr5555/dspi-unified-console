import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SWF4: read-only means read-only everywhere (review c6b N1), and a core
/// state change is applied only to the view of the device it came from (N2).
final class SWF4ReadOnlyTests: XCTestCase {

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    private static func float(_ value: Float) -> Data { withUnsafeBytes(of: value) { Data($0) } }

    private func storedMasterVolume(_ fake: FakeTransport) -> Float? {
        fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)?
            .withUnsafeBytes { $0.load(as: Float.self) }
    }

    /// A view model on the demo device answering with the V17 (1.1.5-beta3)
    /// image, so the core's connection is read-only, and a monitor on its
    /// transport running the view model's own BULK_INVALIDATED hook, as the
    /// app's shared monitor does on the app's transport.
    private func olderFirmware() throws -> (FakeTransport, CoreBackedTransport, DSPViewModel, InterruptMonitor) {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                       length: BULK_PARAMS_SIZE))
        let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: CoreMirrorSW8Tests.v17Image(v32)), .fake))
        let vm = DSPViewModel(transport: core)
        let monitor = InterruptMonitor(transport: core)
        monitor.onBulkInvalidated = AppState.shared.interruptMonitor.onBulkInvalidated   // this view model's
        monitor.start()
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == core.generation },
                      "the connect-time read finishes")
        settle()
        XCTAssertTrue(vm.deviceReadOnly)
        // A connect-time write-as-read may have been refused too; let its
        // notice pass, so what follows is the test's own.
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
        return (fake, core, vm, monitor)
    }

    /// N1: an edit on read-only firmware is put back at once, with no device
    /// notification needed, the status row says why, and the preset is not
    /// left dirty.  The device never changes.
    func testSWF4AnEditOnReadOnlyFirmwareRevertsAtOnceAndSaysWhy() throws {
        let (fake, _, vm, monitor) = try olderFirmware()
        defer { monitor.stop() }
        vm.updateSavedSnapshot()
        XCTAssertEqual(vm.masterVolumeDB, -20)

        // What the sidebar slider does: the value on screen, then the write.
        vm.masterVolumeDB = -3
        vm.sendMasterVolumeToDevice(-3)
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message == DeviceLock.readOnlyMessage }, "the refusal is shown")
        XCTAssertTrue(spinUntil(3) { vm.masterVolumeDB == -20 }, "the device's value is back: \(vm.masterVolumeDB)")
        XCTAssertFalse(vm.hasUnsavedChanges, "no dirty preset for a write that never happened")

        // A setter that writes the view model itself (bypass), and another domain (an EQ band).
        let bypass = vm.bypass
        vm.setBypass(!bypass)
        XCTAssertTrue(spinUntil(3) { vm.bypass == bypass }, "bypass is back")
        let band = try XCTUnwrap(vm.channelData[0]?[0])
        var edited = band
        edited.gain = band.gain + 6
        vm.setFilter(ch: 0, band: 0, p: edited)
        XCTAssertTrue(spinUntil(3) { vm.channelData[0]?[0].gain == band.gain }, "the band is back")

        XCTAssertEqual(storedMasterVolume(fake), -20, "nothing reached the device")
    }

    /// N1: a write-as-read (a preset save) on read-only firmware is refused
    /// with the same wording.
    func testSWF4APresetSaveOnReadOnlyFirmwareIsRefusedWithTheWording() throws {
        let (_, core, _, monitor) = try olderFirmware()
        defer { monitor.stop() }
        let saved = core.getControlResult(request: REQ_PRESET_SAVE, value: 3, index: 2, length: 1)
        XCTAssertEqual(saved, .failure(.denied), "refused, as MIDITransport's read-only mode refuses it")
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message == DeviceLock.readOnlyMessage })
    }

    /// N1: the graph drags no band on read-only firmware (the editor is not
    /// put over the graph), and does on current firmware.
    func testSWF4TheGraphEditsNoBandOnReadOnlyFirmware() throws {
        guard PeqGraphEditorView.isAvailable else { throw XCTSkip("no graph editor on this machine") }
        let (_, _, vm, monitor) = try olderFirmware()
        defer { monitor.stop() }
        vm.activeEqChannel = 0
        vm.channelVisibility[0] = true
        XCTAssertNil(BodePlotView(vm: vm).editedChannel, "no band editor on read-only firmware")

        let demo = CoreBackedTransport(link: .demo)
        let current = DSPViewModel(transport: demo)
        demo.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { current.isDeviceReady && current.fullReadGeneration == demo.generation })
        settle()
        XCTAssertFalse(current.deviceReadOnly)
        current.activeEqChannel = 0
        current.channelVisibility[0] = true
        XCTAssertEqual(BodePlotView(vm: current).editedChannel, 0, "the control: current firmware edits")
    }

    /// The banner says which side to update.
    func testSWF4TheBannerNamesTheSideToUpdate() throws {
        let (_, _, vm, monitor) = try olderFirmware()
        defer { monitor.stop() }
        XCTAssertTrue(vm.firmwareWireFormatVersion < WIRE_FORMAT_VERSION)
        let banner = ReadOnlyBanner(vm: vm)
        XCTAssertTrue(banner.message.contains("older"), banner.message)
        vm.firmwareWireFormatVersion = WIRE_FORMAT_VERSION + 1
        XCTAssertTrue(banner.message.contains("newer"), banner.message)
    }
}

/// A core state source the test drives: its changes are published from
/// another queue, so they reach main one hop later, as the core's do.
private final class ScriptedCoreSource: CoreStateSource {
    let fanout = CoreStateFanout()
    var state: DspState
    init(_ state: DspState) { self.state = state }

    func readAll() -> Result<DspState, DspiError> { .success(state) }
    func readState() -> DspState { state }
    func addStateObserver(_ handler: @escaping (CoreStateChange) -> Void) -> AnyCancellable { fanout.add(handler) }

    /// Publish `state` off main: the handlers run on main, later.
    func publishFromCoreQueue() {
        let change = CoreStateChange(flags: DspiConstants.NOTIFY_STATE, state: state)
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.global().async { self.fanout.publish(change); done.signal() }
        done.wait()
    }
}

final class SWF4MemberSwitchTests: XCTestCase {

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    /// The demo device's state and a view model (never connected) showing it.
    private func shown() -> (DspState, DSPViewModel) {
        let demo = CoreBackedTransport(link: .demo)
        demo.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(10) { demo.isConnected })
        _ = demo.readAll()
        let base = demo.readState()
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        settle()   // its connection sink's first value (attach nil) lands now, not mid-test
        vm.coreMirror.apply(base, force: true)
        return (base, vm)
    }

    /// N2: a change the old member published a main-queue hop before the
    /// switch is dropped; the control shows the same change applies without
    /// the switch.
    func testSWF4AChangeFromTheMemberSwitchedAwayIsNotApplied() {
        let (base, vm) = shown()
        var old = base; old.masterVolumeDb = -7
        let a = ScriptedCoreSource(old), b = ScriptedCoreSource(base)

        vm.coreMirror.attach(a)
        a.publishFromCoreQueue()          // queued on main
        vm.coreMirror.attach(b)           // the switch lands first
        settle()
        XCTAssertEqual(vm.masterVolumeDB, base.masterVolumeDb, "the old member's state reached the new view")

        // The control: the same publication with no switch is applied.
        vm.coreMirror.attach(a)
        a.publishFromCoreQueue()
        settle()
        XCTAssertEqual(vm.masterVolumeDB, -7)
    }

    /// N2: a refused write's revert read from the old member is dropped too.
    func testSWF4ARevertReadFromTheMemberSwitchedAwayIsNotApplied() {
        let (base, vm) = shown()
        var old = base; old.masterVolumeDb = -7
        let a = ScriptedCoreSource(old), b = ScriptedCoreSource(base)

        vm.coreMirror.attach(a)
        vm.coreMirror.revertToCore()      // reads off main, applies on main
        vm.coreMirror.attach(b)
        settle()
        XCTAssertEqual(vm.masterVolumeDB, base.masterVolumeDb)

        vm.coreMirror.attach(a)
        vm.coreMirror.revertToCore()
        XCTAssertTrue(spinUntil(3) { vm.masterVolumeDB == -7 }, "the control: without a switch it applies")
    }
}
