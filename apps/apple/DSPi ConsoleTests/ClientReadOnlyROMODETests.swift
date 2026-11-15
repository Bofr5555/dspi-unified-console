import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP ROMODE: the core's client read-only mode on the core path
/// (`-ReadOnly YES`, and `-MIDIReadOnly YES` once the MIDI link is the
/// core's): every write is refused by the core before it is sent, the edit
/// is put back with the mode's own wording, and reads go on.
final class ClientReadOnlyROMODETests: XCTestCase {

    private func storedMasterVolume(_ fake: FakeTransport) -> Float? {
        fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)?
            .withUnsafeBytes { $0.load(as: Float.self) }
    }

    func testROMODETheMessageIsTheCoresRow() {
        let row = ConsoleNotice.row("read_only.client")
        XCTAssertEqual(row.key, "read_only.client")
        XCTAssertEqual(DeviceLock.clientReadOnlyMessage, "\(row.title): \(row.reason)")
        XCTAssertNotEqual(DeviceLock.clientReadOnlyMessage, DeviceLock.readOnlyMessage)
    }

    func testROMODEAnEditInTheModeIsRefusedByTheCoreAndPutBack() throws {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        let core = CoreBackedTransport(link: .host(fake, .fake), clientReadOnly: true)
        XCTAssertTrue(core.clientReadOnly)
        let vm = DSPViewModel(transport: core)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == core.generation },
                      "the connect-time read finishes: reads go on")
        // A connect-time write-as-read may have been refused; let its notice pass.
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
        let before = try XCTUnwrap(storedMasterVolume(fake))

        vm.masterVolumeDB = before + 3
        vm.sendMasterVolumeToDevice(before + 3)
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message == DeviceLock.clientReadOnlyMessage },
                      "the refusal is shown in the mode's words")
        XCTAssertTrue(spinUntil(3) { vm.masterVolumeDB == before }, "the device's value is back")
        XCTAssertEqual(storedMasterVolume(fake), before, "nothing reached the device")

        // The raw path: a write-as-read is refused, a pure read goes out.
        XCTAssertEqual(core.getControlResult(request: REQ_PRESET_LOAD, value: 3, index: 2, length: 1),
                       .failure(.denied))
        if case .failure(let e) = core.getControlResult(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4) {
            XCTFail("a pure read is refused: \(e)")
        }
    }

    func testROMODEDocumentsAndAPlainLaunchStayWritable() {
        // No `-ReadOnly` in the test host's arguments: off by default, and a
        // document link never takes the launch flag.
        XCTAssertFalse(CoreBackedTransport(link: .document).clientReadOnly)
        XCTAssertEqual(CoreBackedTransport(link: .demo).clientReadOnly, CoreBackedTransport.readOnlyLaunchFlag)
    }
}
