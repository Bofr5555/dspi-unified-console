import XCTest
import dspi_constants
@testable import DSPi_Console

/// WP SWF5: fractional delays survive in the Swift app.  The setters keep the
/// core's 0.0001 ms grid instead of upstream's whole ms, so a 0.5 ms delay
/// from a preset file, another console or the device stays 0.5 ms.  Runs in
/// both test bundles, on the core's demo device.
final class SWF5DelayTests: XCTestCase {

    // MARK: - Helpers

    private func settle(_ seconds: TimeInterval = 0.3) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

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

    private static func float(_ value: Float) -> Data { withUnsafeBytes(of: value) { Data($0) } }

    /// What the device holds: a GET behind every earlier SET on the queue.
    private func deviceDelay(_ t: any DeviceTransport, ch: Int) -> Float? {
        t.getControlRequest(request: REQ_GET_DELAY, value: UInt16(ch), index: 0, length: 4)?
            .withUnsafeBytes { $0.load(as: Float.self) }
    }

    private func deviceOutputDelay(_ t: any DeviceTransport, output: Int) -> Float? {
        t.getControlRequest(request: REQ_GET_OUTPUT_DELAY, value: UInt16(output), index: 2, length: 4)?
            .withUnsafeBytes { $0.load(as: Float.self) }
    }

    // MARK: - Preset import

    /// A preset with 0.5 and 1.5 ms delays on inputs and outputs imports
    /// exactly: the view model and the device hold 0.5 and 1.5, not 1 and 2.
    func testSWF5FractionalPresetDelaysImportExactly() throws {
        let (_, source) = demoVM()
        var doc = PresetDocument.capture(from: source, name: "SWF5")
        var inputs = 0, outputs = 0
        for i in doc.channels.indices {
            if doc.channels[i].outputIndex != nil {
                let ms: Float = outputs % 2 == 0 ? 0.5 : 1.5
                doc.channels[i].delayMs = ms
                doc.channels[i].outputDelayMs = ms
                outputs += 1
            } else if doc.channels[i].inputIndex != nil {
                doc.channels[i].delayMs = inputs % 2 == 0 ? 1.5 : 0.5
                inputs += 1
            }
        }
        XCTAssertGreaterThanOrEqual(inputs, 2)
        XCTAssertGreaterThanOrEqual(outputs, 2)
        let decoded = try PresetDocumentFile.decode(PresetDocumentFile.encode(doc))

        let (t, target) = demoVM()
        var report: PresetApplyReport?
        let options = PresetApplyOptions(audioProcessing: true, volumeLevels: false, hardwareIO: true)
        PresetDocumentApply.apply(decoded, to: target, options: options) { report = $0 }
        XCTAssertTrue(spinUntil(60) { report != nil }, "the apply finishes")
        settle(0.6)
        XCTAssertEqual(report?.isClean, true)

        for block in decoded.channels {
            if let output = block.outputIndex {
                let want = try XCTUnwrap(block.outputDelayMs)
                XCTAssertEqual(target.outputDelayMS[output], want, "output \(output) in the view model")
                XCTAssertEqual(deviceOutputDelay(t, output: output), want, "output \(output) on the device")
            } else if let input = block.inputIndex {
                XCTAssertEqual(target.channelDelays[input], block.delayMs, "input \(input) in the view model")
                XCTAssertEqual(deviceDelay(t, ch: input), block.delayMs, "input \(input) on the device")
            }
        }
        let captured = PresetDocument.capture(from: target)
        XCTAssertEqual(captured.channels.map(\.delayMs), decoded.channels.map(\.delayMs))
        XCTAssertEqual(captured.channels.map(\.outputDelayMs), decoded.channels.map(\.outputDelayMs))
    }

    // MARK: - From the device

    /// A fractional delay another console wrote reaches the view as it is,
    /// and the next unrelated write (the same output's gain) leaves it alone.
    func testSWF5DeviceFractionalDelayIsShownAndKept() {
        let (t, vm) = demoVM()
        // Written past the view model, as another client or a knob does.
        t.sendControlRequest(request: REQ_SET_OUTPUT_DELAY, value: 1, index: 2, data: Self.float(0.5))
        t.sendControlRequest(request: REQ_SET_DELAY, value: 0, index: 0, data: Self.float(1.25))
        XCTAssertTrue(spinUntil(5) { vm.outputDelayMS[1] == 0.5 }, "output delay: \(vm.outputDelayMS[1])")
        XCTAssertTrue(spinUntil(5) { vm.channelDelays[0] == 1.25 }, "input delay: \(vm.channelDelays[0] ?? -1)")
        XCTAssertEqual(ValueField.delayText(vm.outputDelayMS[1]), "0.5")
        XCTAssertEqual(ValueField.delayText(vm.channelDelays[0] ?? 0), "1.25")

        vm.setOutputGain(output: 1, db: -3)
        vm.setOutputMute(output: 1, muted: true)
        settle(0.6)
        XCTAssertEqual(vm.outputDelayMS[1], 0.5)
        XCTAssertEqual(vm.channelDelays[0], 1.25)
        XCTAssertEqual(deviceOutputDelay(t, output: 1), 0.5)
        XCTAssertEqual(deviceDelay(t, ch: 0), 1.25)
    }

    // MARK: - The setters and the text

    /// The setters write what they are given, on the core's grid.
    func testSWF5SettersKeepFractions() {
        let (t, vm) = demoVM()
        vm.setOutputDelay(output: 2, ms: 1.5)
        vm.setDelay(ch: 1, ms: 0.5)
        XCTAssertEqual(vm.outputDelayMS[2], 1.5)
        XCTAssertEqual(vm.channelDelays[1], 0.5)
        XCTAssertEqual(deviceOutputDelay(t, output: 2), 1.5)
        XCTAssertEqual(deviceDelay(t, ch: 1), 0.5)
        settle()
        XCTAssertEqual(vm.outputDelayMS[2], 1.5, "the core's echo keeps it")
        XCTAssertEqual(vm.channelDelays[1], 0.5)

        XCTAssertEqual(DSPViewModel.quantizeDelayMS(0.12344), 0.1234)
        XCTAssertEqual(DSPViewModel.quantizeDelayMS(2.5), 2.5)
        XCTAssertEqual(DSPViewModel.quantizeDelayMS(-0.00001).sign, .plus, "no -0")
    }

    func testSWF5DelayTextTrimsToTheCoresPrecision() {
        XCTAssertEqual(ValueField.delayText(0), "0")
        XCTAssertEqual(ValueField.delayText(12), "12")
        XCTAssertEqual(ValueField.delayText(0.5), "0.5")
        XCTAssertEqual(ValueField.delayText(1.25), "1.25")
        XCTAssertEqual(ValueField.delayText(0.1234), "0.1234")
    }
}
