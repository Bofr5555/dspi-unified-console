import XCTest
import dspi_core
import dspi_ffi
@testable import DSPi_Console

/// WP SW2: the Rust core links into the app on both targets, through the
/// generated bindings (DSPi Console/Core/Generated), and answers.  Runs on
/// the core's demo device; no hardware.
final class CoreBindingsSW2Tests: XCTestCase {

    func testSW2CoreInfo() {
        dspi_ffi.initLogging()
        let info = dspi_core.coreInfo()
        print("SW2: core \(info.ffiVersion), wire \(info.wireFormatVersion) (min \(info.minWireFormatVersion)), bulk \(info.bulkParamsSize), curve \(info.magnitudePoints)")
        XCTAssertFalse(info.ffiVersion.isEmpty)
        XCTAssertGreaterThanOrEqual(info.wireFormatVersion, info.minWireFormatVersion)
        XCTAssertGreaterThan(info.bulkParamsSize, 0)
        XCTAssertGreaterThan(info.magnitudePoints, 0)
    }

    func testSW2OpensTheDemoDevice() throws {
        let core = DspiCore()
        XCTAssertFalse(core.isConnected())
        try core.openDemo()
        XCTAssertTrue(core.isConnected())
        let state = core.getState()
        XCTAssertGreaterThan(state.numChannels, 0)
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        XCTAssertFalse(try core.getDeviceName().isEmpty)
        core.disconnect()
        XCTAssertFalse(core.isConnected())
    }

    /// The app hosting this test opened no real device (TestHostGuard).
    func testSW2TestHostOpensNoRealDevice() throws {
        let env = ProcessInfo.processInfo.environment
        if TestHostGuard.hardwareOptIns.contains(where: { env[$0] == "1" }) {
            throw XCTSkip("This run opted in to real devices")
        }
        XCTAssertTrue(TestHostGuard.underXCTest)
        XCTAssertFalse(TestHostGuard.mayOpenDevices)
        XCTAssertTrue(AppState.shared.midiTransport.availableDevices.isEmpty)
        XCTAssertTrue(AppState.shared.midiCore.availableDevices.isEmpty)   // -- WP SW5 --
        #if os(macOS)
        XCTAssertFalse(AppState.shared.usb.isConnected)
        XCTAssertTrue(AppState.shared.usb.availableDevices.isEmpty)
        #endif
    }
}
