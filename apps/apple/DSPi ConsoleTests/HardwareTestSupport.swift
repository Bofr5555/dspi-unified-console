import XCTest
@testable import DSPi_Console

/// Shared helpers for the DSPi app test suite.
///
/// The suite has two layers:
///  - **pure-logic** tests (`DSPMathTests`, `PresetSnapshotTests`) that need no
///    device and gate cleanly in a device-less CI, and
///  - **live-device** integration tests (`HardwareIntegrationTests`) that drive
///    the real USB command layer against a connected DSPi.
///
/// Live-device tests SKIP (never fail) when no DSPi is attached.
///
/// WP SWF1: a live-device test must leave the device as it found it.
/// `requireDevice` snapshots the raw REQ_GET_ALL_PARAMS image once the app has
/// read the device in full, and the test fails if the image differs after it
/// (`DeviceImageGuard`).  Tests that need no device use `offlineViewModel()`,
/// and tests that would drive a real one through the app's own view model are
/// gated off hardware with `requireNoDevice()`.
enum HardwareTest {
    /// Set once we've waited the full timeout without a device, so a device-less
    /// run skips the remaining tests immediately instead of re-waiting each time.
    private static var probedAbsent = false

    /// Returns the shared USB transport once it reports a live connection, or
    /// throws `XCTSkip` if no device appears within `timeout` seconds.
    ///
    /// The host app's USB hot-plug connection can take ~15 s to establish under
    /// `xcodebuild test`, so the timeout is generous; the result is cached so
    /// the cost is paid at most once per run. The run loop is spun while waiting
    /// because IOKit hot-plug notifications are delivered on the main run loop.
    ///
    /// -- WP SWF1 -- It then waits for the app's view model to finish a full
    /// read of the device, and guards the device image for the rest of `test`.
    static func requireDevice(_ test: XCTestCase, timeout: TimeInterval = 30.0) throws -> CoreUsb {
        // -- WP SW2 -- The test host opens no real device unless the run opts in.
        guard TestHostGuard.mayOpenDevices else {
            throw XCTSkip("Real devices are off under XCTest: set TEST_RUNNER_DSPI_USB_HW=1 to run live-device tests.")
        }
        if probedAbsent {
            throw XCTSkip("No DSPi connected (VID 0x2E8B / PID 0xFEAA) - skipping live-device tests.")
        }

        let usb = AppState.shared.usb
        let transport = AppState.shared.transport

        // "Connected" has to be proven, not read off the published flags.  A test
        // that builds its own view model bounces the device (the init reconnects):
        // the handle closes at once while the flags flip later on the main run
        // loop, so a flag check passes on a device that cannot answer.  The probe
        // is a raw read that only succeeds on an open handle; the transport flag
        // covers the hub re-attaching a few ms after USB reopens.  The run loop
        // is spun while waiting because the reopen arrives through IOKit
        // notifications on the main run loop.
        func linkIsLive() -> Bool {
            usb.isConnected && transport.isConnected
                && usb.getControlRequest(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 6) != nil
        }
        let deadline = Date().addingTimeInterval(timeout)
        while !linkIsLive() && Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
        guard linkIsLive() else {
            probedAbsent = true
            throw XCTSkip("No DSPi connected (VID 0x2E8B / PID 0xFEAA) - skipping live-device tests.")
        }
        // -- WP SWF1 -- The app's own read of the device has to be complete
        // before a test captures from its view model (F1 restored the view
        // model's defaults onto the device), and before the image is taken, so
        // the app's connect-time traffic is not blamed on the test.
        let vm = AppState.shared.viewModel
        guard waitForFullRead(vm, timeout: timeout) else {
            throw HardwareTestFailure("the view model never finished reading the device")
        }
        try guardDeviceImage(in: test) { settle(); return deviceImage(usb) }
        return usb
    }

    // MARK: - WP SWF1: the device image guard

    /// Spins the main run loop until `vm` has finished a full read of the
    /// device its transport has open now (`fullReadGeneration`).  A test that
    /// builds its own view model on the same device bounces it, which starts
    /// a new generation, so a read of the previous one does not count.
    static func waitForFullRead(_ vm: DSPViewModel, timeout: TimeInterval = 30.0) -> Bool {
        func done() -> Bool {
            vm.isDeviceConnected && vm.fullReadGeneration == vm.transport.generation
        }
        let deadline = Date().addingTimeInterval(timeout)
        while !done() && Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
        return done()
    }

    /// The device's whole parameter image (REQ_GET_ALL_PARAMS), read straight
    /// from `device`, not through the view model.
    static func deviceImage(_ device: any DeviceTransport) -> Data? {
        guard let d = device.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                               length: BULK_PARAMS_SIZE),
              d.count == Int(BULK_PARAMS_SIZE) else { return nil }
        return d
    }

    /// Takes the image now and fails `test` if it differs once the test is
    /// over: after its body and its `defer`s (where the restores are), before
    /// `tearDown`.  `read` must order itself after the test's writes.  Throws
    /// if the image cannot be read, since then nothing could be checked.  A
    /// second call in the same test adds nothing.
    static func guardDeviceImage(in test: XCTestCase, read: @escaping () -> Data?) throws {
        guard guardedTest !== test else { return }
        guard let imageGuard = DeviceImageGuard(read: read) else {
            throw HardwareTestFailure("could not read the device image before the test")
        }
        guardedTest = test
        test.addTeardownBlock {
            if let change = imageGuard.check() {
                XCTFail("the test left the device changed: \(change)")
            }
        }
    }
    private static weak var guardedTest: XCTestCase?

    /// A view model on a demo device that is never selected: its setters
    /// update the model and send nothing, so a test of the model's own logic
    /// cannot reach a real device under the hardware opt-ins.  (The default
    /// `DSPViewModel()` drives `AppState.shared.usb`, i.e. the real DSPi when
    /// a run opts in, and bounces it at init.)
    static func offlineViewModel() -> DSPViewModel {
        DSPViewModel(transport: CoreTransportUnderTest.over(FakeTransport(autoSelect: false)))
    }

    /// Skips the test when the run may open a real device.  For tests that
    /// drive the app's shared view model (windows, drags), which is connected
    /// to the real DSPi under the hardware opt-ins.
    static func requireNoDevice() throws {
        if TestHostGuard.mayOpenDevices {
            throw XCTSkip("Drives the app's shared view model, which a hardware run connects to a real DSPi.")
        }
    }

    /// Wait until every write the view model has issued so far has reached the
    /// device.  The view model writes through the DSPi Link hub, whose router
    /// executes the local session's commands in order, so one cheap GET through
    /// the same transport returns only after all earlier writes have run.  The
    /// raw reads below call this first; without it a raw read on the USB serial
    /// queue can overtake a write still crossing the router's queues.
    static func settle() {
        _ = AppState.shared.transport.getControlRequest(request: REQ_GET_PLATFORM, value: 0,
                                                        index: 2, length: 6)
    }

    /// Synchronous little-endian `Float` read of a vendor GET request. This is
    /// an INDEPENDENT decode path - it does not route through the app's
    /// `fetch*`/parse code - so a shared encode/decode bug cannot make a
    /// round-trip falsely pass.  Ordered after the view model's writes by
    /// `settle()`.
    static func readFloat(_ usb: CoreUsb, _ request: UInt8,
                          value: UInt16, index: UInt16 = 0) -> Float? {
        settle()
        guard let d = usb.getControlRequest(request: request, value: value, index: index, length: 4),
              d.count == 4 else { return nil }
        return d.withUnsafeBytes { $0.load(as: Float.self) }
    }

    /// Synchronous unsigned-32 read (EQ type / bypass words are 4-byte ints).
    static func readU32(_ usb: CoreUsb, _ request: UInt8,
                        value: UInt16, index: UInt16 = 0) -> UInt32? {
        settle()
        guard let d = usb.getControlRequest(request: request, value: value, index: index, length: 4),
              d.count == 4 else { return nil }
        return d.withUnsafeBytes { $0.load(as: UInt32.self) }
    }

    /// REQ_GET_EQ_PARAM wValue: bits[15:8]=channel, bits[7:3]=band, bits[2:0]=param
    /// (0=type, 1=freq, 2=Q, 3=gain, 4=bypass).
    static func eqWValue(ch: Int, band: Int, param: Int) -> UInt16 {
        UInt16((ch << 8) | (band << 3) | param)
    }
}

/// WP SWF1: why a live-device test could not start safely (a failure, not a skip).
struct HardwareTestFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

/// WP SWF1: one before/after comparison of a device's parameter image.
final class DeviceImageGuard {
    let before: Data
    private let read: () -> Data?

    /// nil if the image cannot be read.
    init?(read: @escaping () -> Data?) {
        guard let image = read() else { return nil }
        before = image
        self.read = read
    }

    /// nil when the image reads back identical; otherwise what changed.
    func check() -> String? {
        guard let after = read() else { return "the device image could not be read back" }
        return Self.describe(before: before, after: after)
    }

    /// The differing byte ranges, as offsets into the image.
    static func describe(before: Data, after: Data) -> String? {
        let a = [UInt8](before), b = [UInt8](after)
        guard a != b else { return nil }
        guard a.count == b.count else { return "the image is \(b.count) bytes, was \(a.count)" }
        var ranges: [ClosedRange<Int>] = []
        for i in a.indices where a[i] != b[i] {
            if let last = ranges.last, last.upperBound >= i - 4 {
                ranges[ranges.count - 1] = last.lowerBound...i
            } else {
                ranges.append(i...i)
            }
        }
        let differing = a.indices.filter { a[$0] != b[$0] }.count
        let shown = ranges.prefix(12).map { r in
            r.count == 1 ? String(format: "0x%04X", r.lowerBound)
                         : String(format: "0x%04X-0x%04X", r.lowerBound, r.upperBound)
        }
        return "\(differing) bytes of the 0xA0 image differ, at \(shown.joined(separator: ", "))"
            + (ranges.count > 12 ? " and \(ranges.count - 12) more ranges" : "")
    }
}
