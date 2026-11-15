import XCTest
import Combine
import dspi_core
@testable import DSPi_Console

/// WP SW5 on a real DSPi: the core's MIDI transport over CoreMidiPipe, over
/// the development relay (`DSPI_SW5_RELAY=host:port`: how the iOS Simulator
/// reaches a DSPi, `macrelay` on the Mac) or, on macOS without it, CoreMIDI.
/// Opt-in: `TEST_RUNNER_DSPI_MIDI_HW=1 TEST_RUNNER_DSPI_SW5_HW=1`.
///
/// RAM only: reads, typed master-volume writes below the current value,
/// undo and redo, a snapshot capture and recall back to the value found,
/// the device lock.  No preset save, no flash, no VREG, no audio.  The
/// parameter image (0xA0) must read the same at the end as at the start.
final class CoreMidiSW5HardwareTests: XCTestCase {

    private func port() throws -> MIDIControlPort {
        let env = ProcessInfo.processInfo.environment
        guard env["DSPI_MIDI_HW"] == "1", env["DSPI_SW5_HW"] == "1" else {
            throw XCTSkip("Set TEST_RUNNER_DSPI_MIDI_HW=1 and TEST_RUNNER_DSPI_SW5_HW=1 to run against a real DSPi")
        }
        if let spec = env["DSPI_SW5_RELAY"] {
            let parts = spec.split(separator: ":")
            guard parts.count == 2, let p = UInt16(parts[1]) else { throw XCTSkip("DSPI_SW5_RELAY is host:port") }
            return MIDIRelayPort(host: String(parts[0]), port: p)
        }
        #if os(macOS)
        return CoreMIDIControlPort(clientName: "DSPi SW5 hardware test")
        #else
        throw XCTSkip("the Simulator needs DSPI_SW5_RELAY (its CoreMIDI cannot see USB devices)")
        #endif
    }

    private func raw(_ t: CoreBackedTransport, _ request: UInt8, _ value: UInt16 = 0, length: UInt16) -> Data? {
        var out: Data?
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).async {
            if case .success(let d) = t.getControlResult(request: request, value: value, index: 2, length: length) { out = d }
            done.signal()
        }
        while done.wait(timeout: .now()) == .timedOut { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
        return out
    }

    private func volume(_ t: CoreBackedTransport) -> Float? {
        guard let d = raw(t, REQ_GET_MASTER_VOLUME, length: 4), d.count == 4 else { return nil }
        return d.withUnsafeBytes { $0.loadUnaligned(as: Float.self) }
    }

    private func waitFor(_ t: CoreBackedTransport, volume want: Float, _ label: String,
                         file: StaticString = #filePath, line: UInt = #line) {
        let ok = spinUntil(5) { (self.volume(t).map { abs($0 - want) < 0.26 }) ?? false }
        XCTAssertTrue(ok, "\(label): the device reads \(volume(t).map { "\($0)" } ?? "nothing"), want \(want)",
                      file: file, line: line)
        print("SW5-HW \(label): \(volume(t).map { "\($0)" } ?? "?") dB")
    }

    func testSW5HardwareTheCoreOverMIDIReadsWritesUndoesRecallsAndRestores() throws {
        let pipe = CoreMidiPipe(port: try port())
        let t = CoreBackedTransport(link: .midi(pipe))
        guard spinUntil(30, { t.isConnected }) else {
            throw XCTSkip("No DSPi on MIDI: \(t.errorMessage ?? "no device listed")")
        }
        let device = try XCTUnwrap(t.selectedDevice)
        print("SW5-HW device \(device.displayName) serial \(device.serial)")
        let source = try XCTUnwrap(t.coreStateSource, "the core is attached")

        // The image as found: the end must read the same.
        let started = Date()
        let image = try XCTUnwrap(raw(t, REQ_GET_ALL_PARAMS, length: UInt16(BULK_PARAMS_SIZE)), "0xA0 reads")
        XCTAssertEqual(image.count, Int(BULK_PARAMS_SIZE))
        print(String(format: "SW5-HW 0xA0 %d B in %.1f ms through the core", image.count, Date().timeIntervalSince(started) * 1000))
        defer {
            let after = raw(t, REQ_GET_ALL_PARAMS, length: UInt16(BULK_PARAMS_SIZE))
            let differing = after.map { a in zip(a, image).filter { $0 != $1 }.count } ?? -1
            print("SW5-HW image at the end: \(differing == 0 ? "identical" : "\(differing) bytes differ")")
            XCTAssertEqual(after, image, "the device is as found (0xA0)")
            t.disconnect()
        }

        // Read: the core's mirror is the device's.
        let state = try source.readAll().get()
        XCTAssertGreaterThan(state.numOutputChannels, 0)
        let before = try XCTUnwrap(volume(t), "the master volume reads")
        XCTAssertEqual(state.masterVolumeDb, before, accuracy: 0.26, "the core's mirror matches the device")
        print("SW5-HW master volume \(before) dB, \(state.numOutputChannels) outputs, wire features 0x\(String(t.call { $0.wireFeatures() }.sw5Value ?? 0, radix: 16))")

        // A snapshot of the device as found.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("sw5-snap-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        _ = try source.call { try $0.snapshotSetDir(dir: dir.path) }.get()
        let snap = try source.call { try $0.snapshotCapture(name: "SW5 before", note: "") }.get()

        // Typed writes over MIDI; their echoes are our own.
        var own = 0
        let token = t.addNotificationObserver { note in if note.origin == t.session { own += 1 } }
        defer { token.cancel() }
        let a = max(before - 6, -60), b = max(before - 12, -60)
        source.write(lane: nil) { try $0.setMasterVolume(db: a) }
        waitFor(t, volume: a, "first write")
        source.write(lane: nil) { try $0.setMasterVolume(db: b) }
        waitFor(t, volume: b, "second write")
        XCTAssertTrue(spinUntil(5) { own >= 2 }, "the writes' echoes (PARAM_SRC_MIDI) are attributed to us: \(own)")

        // Undo and redo, from the core's journal.
        XCTAssertTrue(spinUntil(5) { source.readJournal().canUndo })
        _ = source.stepJournal(redo: false)
        waitFor(t, volume: a, "undo")
        _ = source.stepJournal(redo: true)
        waitFor(t, volume: b, "redo")

        // The core's device lock (on MIDI it is the core's alone).
        XCTAssertTrue(t.acquireDeviceLock(reason: "SW5 hardware check", timeout: 5))
        t.releaseDeviceLock()

        // Snapshot recall puts the device back as found.
        let recall = try source.call { try $0.snapshotRecall(id: snap.id) }.get()
        print("SW5-HW recall: \(recall.changes.count) changes, image \(recall.image)")
        waitFor(t, volume: before, "snapshot recall")
        _ = try source.call { try $0.snapshotDelete(id: snap.id) }.get()
        XCTAssertEqual(source.readState().masterVolumeDb, before, accuracy: 0.26)

        // Foreground return (iOS): still answers, so a re-read is asked for.
        var resynced = false
        let resync = t.addNotificationObserver { if $0.packet == CoreBackedTransport.resyncPacket { resynced = true } }
        t.resumeMidi()
        XCTAssertTrue(spinUntil(5) { resynced })
        resync.cancel()
        XCTAssertTrue(t.isConnected)
    }
}

private extension Result {
    var sw5Value: Success? {
        if case .success(let s) = self { return s }
        return nil
    }
}
