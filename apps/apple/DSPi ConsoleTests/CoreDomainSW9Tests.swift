import XCTest
import SwiftUI
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW9: volume, EQ, crossover, outputs and matrix through the core's typed
/// setters (Commands.swift), with the slider fast path's write lane
/// (CoreBackedTransport, SWIFT-SEAMS 5.2).  The edits go through the view
/// model as the views make them, and Undo and Redo step them back through the
/// core's journal (CoreUndo.swift).
final class CoreDomainSW9Tests: XCTestCase, CoreDomainTesting {   // -- WP SW10 -- the harness is shared

    private static func masterVolume(_ fake: FakeTransport) -> Float? {
        fake.getControlRequest(request: REQ_GET_MASTER_VOLUME, value: 0, index: 0, length: 4)?
            .withUnsafeBytes { $0.load(as: Float.self) }
    }

    // MARK: - Undo and Redo, one domain at a time

    func testSW9UndoAndRedoAMasterVolumeEdit() {
        let (_, _, vm) = demoVM()
        let before = vm.masterVolumeDB
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        vm.setMasterVolume(before + 6)
        undoAndRedo(vm, undone: { vm.masterVolumeDB == before }, redone: { vm.masterVolumeDB == before + 6 })
    }

    func testSW9UndoAndRedoAUserVolumeAndAnInputPreampEdit() {
        let (_, _, vm) = demoVM()
        vm.linkedInputPairs = 0   // one write, one step
        let user = vm.userVolumeDB
        vm.setUserVolume(user - 10)
        undoAndRedo(vm, undone: { vm.userVolumeDB == user }, redone: { vm.userVolumeDB == user - 10 })

        let pre = vm.preampDB[1]
        vm.setPreampChannel(channel: 1, db: pre - 4)
        undoAndRedo(vm, undone: { vm.preampDB[1] == pre }, redone: { vm.preampDB[1] == pre - 4 })
    }

    func testSW9UndoAndRedoAnEQBandEdit() throws {
        let (core, t, vm) = demoVM()
        let band = try XCTUnwrap(vm.channelData[0]?[2])
        var edited = DSPi_Console.FilterParams(type: .peaking, freq: 2500, q: 1.4, gain: 4.5)
        edited.bypass = false
        vm.setFilter(ch: 0, band: 2, p: edited)
        XCTAssertEqual(vm.channelData[0]?[2], edited)
        undoAndRedo(vm, undone: { vm.channelData[0]?[2] == band }, redone: { vm.channelData[0]?[2] == edited })

        // A band's bypass switch is a step of its own.
        let bypassed = vm.channelData[0]?[2].bypass ?? false
        // -- WP P1SW3F -- the SW9 flake: Undo is already enabled (the band
        // edit), so the helper's Undo could run before the bypass write had
        // reached the journal; it then undid the band edit and the bypass
        // write, landing after it, cleared Redo.  Wait for the journal entry.
        let logged = t.coreQueue.sync { core.history(n: 200).count }
        vm.setBandBypass(ch: 0, band: 2, bypass: !bypassed)
        XCTAssertTrue(spinUntil(5) { t.coreQueue.sync { core.history(n: 200).count } > logged },
                      "the bypass switch is in the journal")
        undoAndRedo(vm, undone: { vm.channelData[0]?[2].bypass == bypassed },
                    redone: { vm.channelData[0]?[2].bypass == !bypassed })
    }

    /// The graph editor's drag: marked with setDragging, a band write per
    /// mouse move through the lane, the commit on release.  One Undo puts
    /// the band back as it was before the drag.
    func testSW9AGraphDragIsOneUndoStep() throws {
        let (core, t, vm) = demoVM()
        let before = try XCTUnwrap(vm.channelData[0]?[3])
        var p = DSPi_Console.FilterParams(type: .peaking, freq: 1000, q: 1, gain: 0)
        t.setDragging(true)
        for step in 1...12 {
            p.freq = 1000 + Float(step) * 50
            p.gain = Float(step) * 0.5
            vm.sendFilterToDevice(ch: 0, band: 3, p: p)
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        vm.setFilter(ch: 0, band: 3, p: p)   // the release
        t.setDragging(false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no band write went out raw")

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[3] == before },
                      "the whole drag in one step: \(String(describing: vm.channelData[0]?[3]))")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo }, "nothing else to undo")
        vm.undo.redo()
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[3] == p }, "the drag's end again")
    }

    func testSW9UndoAndRedoACrossoverEdit() throws {
        let (_, _, vm) = demoVM()
        let ch = vm.chOut1
        let band = try XCTUnwrap(vm.xoverData[ch]?[0])
        let edited = DSPi_Console.FilterParams(type: .lr4_lp, freq: 1200, q: 0.707, gain: 0)
        vm.setCrossoverBand(ch: ch, localBand: 0, p: edited)
        undoAndRedo(vm, undone: { vm.xoverData[ch]?[0] == band }, redone: { vm.xoverData[ch]?[0] == edited })
    }

    func testSW9UndoAndRedoOutputEdits() {
        let (_, _, vm) = demoVM()
        let gain = vm.outputGainDB[1]
        vm.setOutputGain(output: 1, db: gain - 3)
        undoAndRedo(vm, undone: { vm.outputGainDB[1] == gain }, redone: { vm.outputGainDB[1] == gain - 3 })

        let muted = vm.outputMuted[1]
        vm.setOutputMute(output: 1, muted: !muted)
        undoAndRedo(vm, undone: { vm.outputMuted[1] == muted }, redone: { vm.outputMuted[1] == !muted })

        let delay = vm.outputDelayMS[2]
        vm.setOutputDelay(output: 2, ms: delay + 3)
        undoAndRedo(vm, undone: { vm.outputDelayMS[2] == delay }, redone: { vm.outputDelayMS[2] == delay + 3 })
    }

    func testSW9UndoAndRedoAMatrixEdit() {
        let (_, _, vm) = demoVM()
        let (on, gain, inv) = (vm.matrixRouting[0][1], vm.matrixGain[0][1], vm.matrixInvert[0][1])
        vm.setMatrixRoute(input: 0, output: 1, enabled: !on, gain: -6, invert: !inv)
        undoAndRedo(vm,
                    undone: { vm.matrixRouting[0][1] == on && vm.matrixGain[0][1] == gain && vm.matrixInvert[0][1] == inv },
                    redone: { vm.matrixRouting[0][1] == !on && vm.matrixGain[0][1] == -6 && vm.matrixInvert[0][1] == !inv })
    }

    /// A slider drag on the master volume: many lane writes, one step.
    func testSW9AVolumeSliderDragIsOneUndoStep() {
        let (_, t, vm) = demoVM()
        let before = vm.masterVolumeDB
        t.setDragging(true)
        for step in 1...10 {
            let v = before + Float(step)
            vm.masterVolumeDB = v
            vm.sendMasterVolumeToDevice(v)
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        vm.setMasterVolume(before + 10)
        t.setDragging(false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == before }, "one step: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo })
    }

    // MARK: - Drag marks ended when a view goes (review lk1 S1)

    /// A control's mark: set once, ended once, a repeat ignored.
    func testSW9ADragMarkIsSetAndEndedOnce() {
        var marks: [Bool] = []
        let mark = CoreDragMark { marks.append($0) }
        mark.set(true); mark.set(true)
        XCTAssertTrue(mark.isOn)
        mark.end(); mark.end(); mark.set(false)
        XCTAssertEqual(marks, [true, false])
        XCTAssertFalse(mark.isOn)
    }

    /// A SwiftUI control that leaves the screen mid-drag (a sheet
    /// dismissed, a navigation pop, a tool window closed) gets no
    /// `onEditingChanged(false)`; `endsCoreDrag` ends its mark.
    func testSW9AViewGoneMidDragEndsItsMark() throws {
        var marks: [Bool] = []
        var appeared = false
        let mark = CoreDragMark { marks.append($0) }
        let view = Text("slider").onAppear { appeared = true }.endsCoreDrag(mark)
        #if os(macOS)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 200, height: 100),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = NSHostingView(rootView: view)
        window.orderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(spinUntil(5) { appeared }, "on screen")
        mark.set(true)                 // a drag begins
        window.contentView = nil       // and the view goes with it
        #else
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 200, height: 100)
        window.rootViewController = UIHostingController(rootView: view)
        window.isHidden = false
        defer { window.isHidden = true }
        XCTAssertTrue(spinUntil(5) { appeared }, "on screen")
        mark.set(true)
        window.rootViewController = nil
        #endif
        XCTAssertTrue(spinUntil(5) { marks == [true, false] }, "the mark is ended: \(marks)")
    }

    /// What a mark left on did: every later edit joined the drag's step.
    /// Ended, the edits after it are steps of their own.
    func testSW9EditsAfterAnEndedMarkAreStepsOfTheirOwn() {
        let (_, t, vm) = demoVM()
        let before = vm.masterVolumeDB
        let mark = CoreDragMark { t.setDragging($0) }
        mark.set(true)
        vm.sendMasterVolumeToDevice(before + 2)
        vm.setMasterVolume(before + 2)
        mark.end()                         // the view went mid-drag
        vm.setMasterVolume(before + 6)     // a later edit elsewhere
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo && vm.masterVolumeDB == before + 6 })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == before + 2 },
                      "the later edit is its own step: \(vm.masterVolumeDB)")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == before }, "then the drag: \(vm.masterVolumeDB)")
    }

    // MARK: - Fixup sw9

    /// F1 (with the core's P0W2): a graph drag whose motion changes kind
    /// mid-way (gain only, then frequency and gain) is still one Undo step.
    func testSW9AMixedMotionGraphDragIsOneUndoStep() throws {
        let (_, t, vm) = demoVM()
        var p = DSPi_Console.FilterParams(type: .peaking, freq: 1000, q: 1, gain: 0)
        vm.setFilter(ch: 0, band: 3, p: p)            // a known start, a step of its own
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[3] == p })
        let start = p
        t.setDragging(true)
        for step in 1...5 {                           // vertical: gain only
            p.gain = Float(step) * 0.5
            vm.sendFilterToDevice(ch: 0, band: 3, p: p)
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        for step in 1...5 {                           // diagonal: frequency and gain
            p.freq = 1000 + Float(step) * 50
            p.gain = 2.5 + Float(step) * 0.5
            vm.sendFilterToDevice(ch: 0, band: 3, p: p)
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        vm.setFilter(ch: 0, band: 3, p: p)            // the release
        t.setDragging(false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[3] == start },
                      "one Undo puts the band back where the drag began: \(String(describing: vm.channelData[0]?[3]))")
        vm.undo.redo()
        XCTAssertTrue(spinUntil(5) { vm.channelData[0]?[3] == p }, "and Redo to where it ended")
    }

    /// F2: a lane value issued after a commit of the same setting is sent
    /// after it (the review's probe P2); the commit supersedes the value
    /// still queued before it.
    func testSW9ALaneValueAfterACommitIsNotOvertaken() {
        let (core, t, vm) = demoVM()
        let busy = DispatchSemaphore(value: 0)
        t.coreQueue.async { busy.wait() }
        vm.sendMasterVolumeToDevice(-30)   // a lane value, queued
        vm.setMasterVolume(-25)            // the commit: -30 is superseded (the demo starts at -20)
        vm.sendMasterVolumeToDevice(-10)   // newer than the commit
        busy.signal()
        flush(t)
        XCTAssertEqual(t.readState().masterVolumeDb, -10, "the newest value issued is the device's")
        let sent = entries(core, t, "vol.master")
        XCTAssertEqual(sent.count, 2, "the commit, then the newer value; -30 never sent: \(sent.map(\.after))")
    }

    /// F3: a crosspoint gain is kept on the core's 0.1 dB grid, so the value
    /// shown at once is the device's and nothing snaps after the window.
    func testSW9MatrixGainFollowsTheCoresGrid() {
        let (_, t, vm) = demoVM()
        vm.setMatrixRoute(input: 0, output: 1, enabled: true, gain: -4.25, invert: false)
        let shownAtOnce = vm.matrixGain[0][1]
        flush(t)
        let kept = t.readState().matrixGain[0][1]
        XCTAssertEqual(kept, -4.3)
        XCTAssertEqual(shownAtOnce, kept)
        settle(CoreMirror.defaultPendingWindow + 0.3)
        XCTAssertEqual(vm.matrixGain[0][1], kept)
    }

    // MARK: - No raw writes in this domain

    /// Every setter of the domain goes to a typed core call: the journal
    /// holds no raw entry (a raw SET is journalled as "raw"), and a re-read
    /// of the device (Swift's FakeTransport, through the core) finds the
    /// values: the wire reached the device.
    func testSW9EveryDomainWriteIsTypedAndReachesTheDevice() throws {
        let (_, core, t, vm) = hostVM()
        vm.linkedInputPairs = 0
        let peq = DSPi_Console.FilterParams(type: .peaking, freq: 3150, q: 2, gain: -3.5)
        let xover = DSPi_Console.FilterParams(type: .lr4_hp, freq: 90, q: 0.707, gain: 0)
        let out = vm.chOut1

        vm.setMasterVolume(-12.5)
        vm.sendMasterVolumeToDevice(-11)
        vm.setMasterVolume(-10)
        vm.setUserVolume(-7)
        vm.sendUserVolumeToDevice(-6)
        vm.setUserVolume(-5)
        vm.setPreamp(-2)
        vm.sendPreampChannelToDevice(channel: 1, db: -1)
        vm.setPreampChannel(channel: 1, db: -1.5)
        vm.setFilter(ch: 0, band: 1, p: peq)
        vm.sendFilterToDevice(ch: 1, band: 1, p: peq)
        vm.setBandBypass(ch: 0, band: 4, bypass: true)
        vm.setCrossoverBand(ch: out, localBand: 1, p: xover)
        vm.setCrossoverBandBypass(ch: out, localBand: 2, bypass: true)
        vm.setDelay(ch: 0, ms: 1.25)
        vm.setBypass(!vm.bypass)
        let bypass = vm.bypass
        vm.setMatrixRoute(input: 1, output: 0, enabled: true, gain: -4, invert: true)
        vm.sendOutputGainToDevice(output: 0, db: -2)
        vm.setOutputGain(output: 0, db: -2.5)
        vm.setOutputMute(output: 1, muted: true)
        vm.sendOutputDelayToDevice(output: 2, ms: 2)
        vm.setOutputDelay(output: 2, ms: 2.5)
        flush(t)

        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no write of this domain went out raw")

        // A fresh read of the device through the core.
        guard case .success(let s) = t.readAll() else { return XCTFail("re-read") }
        XCTAssertEqual(s.masterVolumeDb, -10)
        XCTAssertEqual(s.userVolumeDb, -5)
        XCTAssertEqual(s.preampDb, -2, "the legacy all-input preamp (the Swift fake keeps its own field)")
        XCTAssertEqual(s.inputPreampDb[1], -1.5)
        XCTAssertEqual(s.filters[0][1].filterType, UInt8(peq.type.rawValue))
        XCTAssertEqual(s.filters[0][1].freq, 3150)
        XCTAssertEqual(s.filters[0][1].gain, -3.5)
        XCTAssertEqual(s.filters[1][1].freq, 3150, "the drag-time write reached the device")
        XCTAssertTrue(s.filters[0][4].bypass)
        XCTAssertEqual(s.xover[out][1].filterType, UInt8(xover.type.rawValue))
        XCTAssertEqual(s.xover[out][1].freq, 90)
        XCTAssertTrue(s.xover[out][2].bypass)
        XCTAssertEqual(s.channelDelays[0], 1.25)
        XCTAssertEqual(s.bypass, bypass)
        XCTAssertTrue(s.matrixRouting[1][0])
        XCTAssertEqual(s.matrixGain[1][0], -4)
        XCTAssertTrue(s.matrixInvert[1][0])
        XCTAssertEqual(s.outputGainDb[0], -2.5)
        XCTAssertTrue(s.outputMuted[1])
        XCTAssertEqual(s.outputDelayMs[2], 2.5)
    }

    /// Output latency and Core 1 through the core's typed reads (the demo
    /// device answers both).
    func testSW9OutputLatencyAndCore1ModeThroughTheCore() {
        let (_, _, vm) = demoVM()
        DispatchQueue.global().async { vm.fetchOutputLatency(); vm.fetchCore1Mode() }
        XCTAssertTrue(spinUntil(5) { vm.outputLatencySupported && !vm.outputLatencyOptions.isEmpty },
                      "the status and the option table")
        XCTAssertEqual(Int(vm.outputLatencyStatus.optionCount), vm.outputLatencyOptions.count)
    }

    // MARK: - The write lane (SWIFT-SEAMS 5.2)

    /// While a value for a key is still queued, a newer one replaces it: a
    /// burst sends one write, the newest.
    func testSW9TheLaneSendsOnlyTheNewestQueuedValue() {
        let (core, t, vm) = demoVM()
        let busy = DispatchSemaphore(value: 0)
        t.coreQueue.async { busy.wait() }
        for step in 1...20 { vm.sendMasterVolumeToDevice(-20 + Float(step) * 0.5) }
        busy.signal()
        let sent = entries(core, t, "vol.master")
        XCTAssertEqual(sent.count, 1, "one write for the burst")
        XCTAssertEqual(t.readState().masterVolumeDb, -10, "the newest value")

        // Different keys are not merged.
        t.coreQueue.async { busy.wait() }
        vm.sendOutputGainToDevice(output: 0, db: -1)
        vm.sendOutputGainToDevice(output: 1, db: -2)
        busy.signal()
        XCTAssertEqual(entries(core, t, "out.gain").count, 2)
    }

    /// A lane write issued for a connection that has gone is dropped, never
    /// sent to the next one.  The control: the same write without the
    /// switch lands.
    func testSW9ALaneWriteForAnEarlierConnectionIsDropped() {
        let (fake, _, t, vm) = hostVM()
        let before = Self.masterVolume(fake)
        let busy = DispatchSemaphore(value: 0)
        t.coreQueue.async { busy.wait() }
        vm.sendMasterVolumeToDevice(-3)
        fake.disconnect()                 // the generation moves at once
        busy.signal()
        flush(t)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "connected and read again")
        flush(t)
        XCTAssertEqual(Self.masterVolume(fake), before, "dropped, not replayed")

        vm.sendMasterVolumeToDevice(-3)
        flush(t)
        XCTAssertEqual(Self.masterVolume(fake), -3, "the control: it lands")
    }

    /// A failed write drops every queued lane value (nothing is replayed),
    /// and a value the core refuses says why and is put back.
    func testSW9AFailedWriteDropsTheQueuedLane() {
        let (core, t, vm) = demoVM()
        let before = vm.masterVolumeDB
        let busy = DispatchSemaphore(value: 0)
        t.coreQueue.async { busy.wait() }
        vm.sendOutputGainToDevice(output: 200, db: -1)   // no such output: the core refuses
        vm.sendMasterVolumeToDevice(before + 5)
        busy.signal()
        flush(t)
        XCTAssertTrue(entries(core, t, "vol.master").isEmpty, "queued behind the failure: dropped")
        XCTAssertEqual(t.readState().masterVolumeDb, before)
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message?.contains("200") == true },
                      "the core's reason: \(DeviceLock.notice.message ?? "none")")
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
    }

    /// Without a core (USB without -CoreTransport, MIDI, the Link client)
    /// the same setters stay raw SETs, so those paths work as before.
    func testSW9WithoutACoreTheSettersStayRaw() {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == fake.generation })
        settle()
        XCTAssertNil(vm.typedCore)
        vm.setMasterVolume(-7.3)   // this app's finer step near 0 dB
        XCTAssertTrue(spinUntil(3) { Self.masterVolume(fake) == -7.3 })
    }

    /// Through a core, the master volume and a band's gain are kept on the
    /// core's grid, so the value shown is the value the device gets.
    func testSW9ValuesFollowTheCoresGrid() {
        let (_, t, vm) = demoVM()
        vm.setMasterVolume(-7.3)
        XCTAssertEqual(vm.masterVolumeDB, -7.5)
        var p = DSPi_Console.FilterParams(type: .peaking, freq: 1000, q: 1, gain: 2.345)
        p.bypass = false
        vm.setFilter(ch: 0, band: 5, p: p)
        XCTAssertEqual(vm.channelData[0]?[5].gain, 2.3)
        flush(t)
        XCTAssertEqual(t.readState().masterVolumeDb, -7.5)
        XCTAssertEqual(t.readState().filters[0][5].gain, 2.3)
    }
}
