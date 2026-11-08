import XCTest
@testable import DSPi_Console

/// WP SWF1: the live-device tests' image guard (`HardwareTest.guardDeviceImage`,
/// `DeviceImageGuard`), proven on the demo device (FakeTransport), so it runs
/// in every pass without a DSPi.  The last two tests replay F1 from the
/// hardware validation: `PresetDocumentHardwareTests` captured the view model
/// before it had read the device and "restored" its defaults onto it.
final class DeviceImageGuardTests: XCTestCase {

    /// The demo device, open, under a view model that has read it in full.
    private func demo(waitForFullRead: Bool = true) -> (FakeTransport, DSPViewModel) {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        if waitForFullRead {
            XCTAssertTrue(HardwareTest.waitForFullRead(vm, timeout: 10),
                          "the view model never read the demo device in full")
        }
        return (fake, vm)
    }

    private func setOutputGain(_ fake: FakeTransport, _ db: Float) {
        var v = db
        fake.sendControlRequest(request: REQ_SET_OUTPUT_GAIN, value: 0, index: 2, data: Data(bytes: &v, count: 4))
    }

    private func outputGain(_ fake: FakeTransport) -> Float? {
        HardwareTest.deviceImage(fake).map {
            $0.withUnsafeBytes { $0.load(fromByteOffset: BULK_OUTPUTS_OFFSET + 4, as: Float.self) }
        }
    }

    /// Applies a document and waits for it to finish (as the hardware test does).
    private func apply(_ doc: PresetDocument, to vm: DSPViewModel) throws {
        let finished = expectation(description: "configuration applied")
        PresetDocumentApply.apply(doc, to: vm, options: PresetApplyOptions()) { _ in finished.fulfill() }
        wait(for: [finished], timeout: 30)
    }

    func testSWF1UntouchedDeviceRaisesNothing() throws {
        let (fake, _) = demo()
        let imageGuard = try XCTUnwrap(DeviceImageGuard { HardwareTest.deviceImage(fake) })
        XCTAssertEqual(imageGuard.before.count, Int(BULK_PARAMS_SIZE))
        XCTAssertNil(imageGuard.check())
    }

    func testSWF1ChangeLeftBehindIsReported() throws {
        let (fake, _) = demo()
        let imageGuard = try XCTUnwrap(DeviceImageGuard { HardwareTest.deviceImage(fake) })
        let original = try XCTUnwrap(outputGain(fake))
        setOutputGain(fake, original - 3)
        let change = try XCTUnwrap(imageGuard.check(), "a changed gain must be reported")
        // The report starts at the gain's first changed byte.
        let was = withUnsafeBytes(of: original) { [UInt8]($0) }
        let now = withUnsafeBytes(of: original - 3) { [UInt8]($0) }
        let first = try XCTUnwrap(was.indices.first { was[$0] != now[$0] })
        XCTAssertTrue(change.contains("of the 0xA0 image differ, at "
                                      + String(format: "0x%04X", BULK_OUTPUTS_OFFSET + 4 + first)), change)
    }

    func testSWF1RestoredChangeRaisesNothing() throws {
        let (fake, _) = demo()
        let imageGuard = try XCTUnwrap(DeviceImageGuard { HardwareTest.deviceImage(fake) })
        let original = try XCTUnwrap(outputGain(fake))
        setOutputGain(fake, original - 3)
        XCTAssertNotNil(imageGuard.check())
        setOutputGain(fake, original)
        XCTAssertNil(imageGuard.check())
    }

    /// End to end: the guard `requireDevice` installs fails the test that
    /// leaves the device changed, after the test body has returned.
    func testSWF1GuardFailsTheTestThatLeavesTheDeviceChanged() throws {
        let (fake, _) = demo()
        try HardwareTest.guardDeviceImage(in: self) { HardwareTest.deviceImage(fake) }
        XCTExpectFailure("the guard must fail this test: the gain is not restored") { issue in
            issue.compactDescription.contains("the test left the device changed")
        }
        setOutputGain(fake, (outputGain(fake) ?? 0) - 3)
    }

    /// F1 as it was: a capture before the view model's full read holds its
    /// defaults, and applying it as the "original" changes the device.
    func testSWF1CaptureBeforeTheFullReadIsCaught() throws {
        let (fake, vm) = demo(waitForFullRead: false)
        let original = PresetDocument.capture(from: vm)
        XCTAssertTrue(HardwareTest.waitForFullRead(vm, timeout: 10))
        let imageGuard = try XCTUnwrap(DeviceImageGuard { HardwareTest.deviceImage(fake) })
        try apply(original, to: vm)
        XCTAssertNotNil(imageGuard.check(), "restoring a capture of the defaults must change the demo device")
    }

    /// F1 fixed: captured after the full read, changed, applied, and the
    /// original applied again, the device's image is as it was.
    func testSWF1CaptureAfterTheFullReadRestoresTheDevice() throws {
        let (fake, vm) = demo()
        let imageGuard = try XCTUnwrap(DeviceImageGuard { HardwareTest.deviceImage(fake) })
        let original = PresetDocument.capture(from: vm)

        var modified = original
        modified.global.inputPreampsDb[0] = -7.5
        let platform = vm.platformName
        let outputIndex = try XCTUnwrap(modified.channels.firstIndex { $0.ref(platform: platform) == .output(0) })
        modified.channels[outputIndex].gainDb = -5.5
        modified.channels[outputIndex].outputDelayMs = 3
        try apply(modified, to: vm)
        XCTAssertNotNil(imageGuard.check(), "the modified document did not reach the demo device")

        try apply(original, to: vm)
        XCTAssertNil(imageGuard.check())
    }
}
