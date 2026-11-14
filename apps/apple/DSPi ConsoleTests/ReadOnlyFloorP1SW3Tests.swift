import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP P1SW3 (PRO1 S8): the read-only floor.  On read-only firmware (the V17,
/// V26, V31 and V33 fixtures) and on a demo device the installer has locked,
/// the main window stays up for reading; what the state allows stays live and
/// the rest is off with the core's reason.  The floor's table is checked
/// against what the core itself lets through.  In the macOS and iOS bundles.
final class ReadOnlyFloorP1SW3Tests: XCTestCase, CoreDomainTesting {

    private var folders: [URL] = []

    override func tearDown() {
        for f in folders { try? FileManager.default.removeItem(at: f) }
        folders = []
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        super.tearDown()
    }

    private static let all: [FloorControl] = [.volume, .mute, .presetRecall, .analyser, .other]

    // MARK: - The table

    func testP1SW3TheFloorsTable() {
        let open = ReadOnlyFloor.of(connected: true, deviceReadOnly: false, lockedUser: false)
        XCTAssertEqual(open, .open)
        XCTAssertTrue(Self.all.allSatisfy(open.allows))
        XCTAssertEqual(open.reason, "")

        let fw = ReadOnlyFloor.of(connected: true, deviceReadOnly: true, lockedUser: false)
        XCTAssertEqual(fw.cause, .readOnlyFirmware)
        XCTAssertEqual(Self.all.filter(fw.allows), [.analyser], "read-only firmware: reading only")
        XCTAssertEqual(fw.reason, noticeForKey(key: "read_only", detail: "").reason, "the core's reason")

        let locked = ReadOnlyFloor.of(connected: true, deviceReadOnly: false, lockedUser: true)
        XCTAssertEqual(locked.cause, .installerLock)
        XCTAssertEqual(Self.all.filter(locked.allows), [.volume, .mute, .presetRecall, .analyser],
                       "the lock: the core's LOCKED_USER_WRITES and the analyser")
        XCTAssertEqual(locked.reason, noticeForKey(key: "installer_lock.banner", detail: "").reason)

        XCTAssertEqual(ReadOnlyFloor.of(connected: true, deviceReadOnly: true, lockedUser: true).cause,
                       .readOnlyFirmware, "the firmware allows less")
        XCTAssertEqual(ReadOnlyFloor.of(connected: false, deviceReadOnly: true, lockedUser: true), .open,
                       "no device: nothing to keep from")
    }

    // MARK: - A locked demo device

    private func folder(_ name: String) -> URL {
        let url = TestFolders.fresh("P1SW3-\(name)")   // -- WP P1SW3F -- F5: the caches directory
        folders.append(url)
        return url
    }

    func testP1SW3TheFloorOnALockedDemoDeviceIsWhatTheCoreAllows() {
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        let (_, t, vm) = demoVM()
        let installer = CoreInstaller(vm: vm, folder: folder("limits"))
        XCTAssertTrue(spinUntil(10) { installer.available && !installer.busy && installer.choice != nil })
        XCTAssertEqual(ReadOnlyFloor.of(vm, installer: installer), .open, "no PIN: nothing is off")

        installer.setPin("1357", again: "1357")
        XCTAssertTrue(spinUntil(10) { installer.lock.pinSet })
        installer.setCovered(true)
        XCTAssertTrue(spinUntil(5) { !installer.lock.covers.isEmpty })
        installer.lockNow()
        XCTAssertTrue(spinUntil(10) { installer.lockedUser }, "locked")

        let floor = ReadOnlyFloor.of(vm, installer: installer)
        XCTAssertEqual(floor.cause, .installerLock)
        XCTAssertFalse(vm.deviceReadOnly, "the firmware is current: the lock is the console's")
        XCTAssertEqual(floor.reason, CoreInstaller.reason("installer_lock.banner"),
                       "the controls that are off say the banner's reason")

        // What the floor leaves live, the core lets through ...
        XCTAssertTrue(floor.allows(.volume))
        let v = t.readState().masterVolumeDb
        vm.setMasterVolume(v - 2)
        XCTAssertTrue(spinUntil(5) { t.readState().masterVolumeDb == v - 2 }, "volume passes")
        XCTAssertTrue(floor.allows(.mute))
        vm.setOutputMute(output: 0, muted: true)
        XCTAssertTrue(spinUntil(5) { t.readState().outputMuted.first == true }, "an output's mute passes")
        vm.setOutputMute(output: 0, muted: false)
        XCTAssertTrue(spinUntil(5) { t.readState().outputMuted.first == false })
        XCTAssertTrue(floor.allows(.presetRecall))
        XCTAssertNotEqual(vm.loadPreset(slot: 0), PRESET_STATUS_LOCKED, "a preset recall is not refused by the lock")

        // ... and what it turns off, the core would refuse after the fact.
        XCTAssertFalse(floor.allows(.other))
        let gain = t.readState().outputGainDb[1]
        vm.setOutputGain(output: 1, db: gain - 3)
        flush(t)
        XCTAssertTrue(spinUntil(5) { t.readState().outputGainDb[1] == gain }, "a setting is refused by the core")
        XCTAssertNotEqual(vm.savePreset(slot: 1, confirmed: .savePreset()), PRESET_OK, "a save is refused")

        // The graph drags no band under the lock (its editor goes by the
        // shared installer, which this test does not lock: the table is the
        // same call with this one).
        XCTAssertFalse(ReadOnlyFloor.of(vm, installer: installer).allows(.other))

        // Unlocked: everything is live again.
        var ok: Bool?
        installer.unlock("1357") { ok = $0 }
        XCTAssertTrue(spinUntil(10) { ok != nil })
        XCTAssertTrue(spinUntil(5) { !installer.lockedUser })
        XCTAssertEqual(ReadOnlyFloor.of(vm, installer: installer), .open)
    }

    // MARK: - The read-only fixtures

    /// The demo's V32 image as firmware of wire format `wire` lays it out:
    /// Swift's copy of the core's fixtures (`dspi-transport` fake/wire_images.rs,
    /// QD6).  V17, V26 and V31 are older, V33 newer.
    static func fixtureImage(_ v32: Data, wire: UInt8) -> Data? {
        let sizes: [UInt8: Int] = [17: 5872, 26: 5944, 31: 6028]
        var image: [UInt8]
        if wire == 33 {
            // A newer firmware appends: the V32 prefix, then what it added.
            image = [UInt8](v32) + [UInt8](repeating: 0xEE, count: 16)
        } else if wire == 17 {
            return CoreMirrorSW8Tests.v17Image(v32)
        } else if let size = sizes[wire] {
            var laid = [UInt8](v32)
            let ic = BULK_INPUT_CONFIG_OFFSET
            if wire < 28 {
                // Before V28 the input config's tail sat one byte earlier.
                let tail = Array(laid[(ic + 11)..<(ic + 16)])
                laid.replaceSubrange((ic + 10)..<(ic + 15), with: tail)
            }
            image = Array(laid.prefix(size))
        } else {
            return nil
        }
        image[0] = wire
        image[6] = UInt8(image.count & 0xFF)
        image[7] = UInt8(image.count >> 8)
        return Data(image)
    }

    func testP1SW3TheV17V26V31AndV33FixturesAreReadOnlyOnTheFloor() throws {
        for wire: UInt8 in [17, 26, 31, 33] {
            let fake = FakeTransport(autoSelect: false)
            fake.selectDevice(FakeTransport.demoDevice)
            let v32 = try XCTUnwrap(fake.getControlRequest(request: REQ_GET_ALL_PARAMS, value: 0, index: 2,
                                                           length: BULK_PARAMS_SIZE))
            let image = try XCTUnwrap(Self.fixtureImage(v32, wire: wire))
            let core = CoreBackedTransport(link: .host(OlderImageTransport(fake, image: image), .fake))
            let vm = DSPViewModel(transport: core)
            XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == core.generation },
                          "V\(wire): the connect-time read finishes")
            settle()

            XCTAssertTrue(vm.deviceReadOnly, "V\(wire) is read-only")
            XCTAssertEqual(vm.firmwareWireFormatVersion, Int(wire))
            XCTAssertTrue(vm.isDeviceReady, "V\(wire): the values are shown")
            XCTAssertEqual(vm.masterVolumeDB, -20, "V\(wire): read through the core's mapping")
            XCTAssertEqual(vm.channelNames[0], "USB 1", "V\(wire)")

            let floor = ReadOnlyFloor.of(connected: vm.isDeviceConnected, deviceReadOnly: vm.deviceReadOnly,
                                         lockedUser: false)
            XCTAssertEqual(floor.cause, .readOnlyFirmware, "V\(wire)")
            XCTAssertEqual(Self.all.filter(floor.allows), [.analyser], "V\(wire): reading only")
            XCTAssertEqual(floor.reason, noticeForKey(key: "read_only", detail: "").reason)
            #if os(iOS)
            XCTAssertEqual(FileMenuActions.importRefusal(vm, needsDevice: true)?.key, "read_only",
                           "V\(wire): an import is refused up front with the core's row")
            #endif

            // What the floor turns off, the core would refuse: nothing reaches the device.
            vm.masterVolumeDB = -3
            vm.sendMasterVolumeToDevice(-3)
            flush(core)
            let stored = fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)
            XCTAssertEqual(stored?.withUnsafeBytes { $0.load(as: Float.self) }, -20, "V\(wire): nothing written")
            XCTAssertEqual(core.readState().masterVolumeDb, -20, "V\(wire): the core keeps the device's value")
        }
    }

    // MARK: - Stale meters

    func testP1SW3TheMetersSayStaleSince() {
        XCTAssertEqual(SystemStatus.staleLine(since: nil), "")
        let since = Date(timeIntervalSince1970: 1_000_000)
        let line = SystemStatus.staleLine(since: since, now: since.addingTimeInterval(12))
        XCTAssertTrue(line.hasPrefix("Meters stale since "), line)
        XCTAssertTrue(line.hasSuffix("(12 s ago): the last reading failed"), line)   // -- WP P1SW3F -- F5
    }
}
