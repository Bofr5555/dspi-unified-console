import XCTest
import dspi_core
@testable import DSPi_Console

/// WP SWS7: the effect sliders' live sends are keyed by the setting (the
/// core's registry path, `DSPViewModel.LiveFloat`), not by an opcode.  On the
/// core a drag, marked as `ParameterRow` marks it and committed by the
/// setting's `set*` on release, is one undo step; without a core
/// (`-CoreTransport NO`) the same send is still the raw SET.
final class LiveSendsSWS7Tests: XCTestCase, CoreDomainTesting {

    /// One live setting: its view-model setter (the commit on release), what
    /// the view shows, and three values (a known start, a value the drag
    /// passes through, the value released on).
    private struct Case {
        let setting: DSPViewModel.LiveFloat
        let commit: (Float) -> Void
        let shown: () -> Float
        let values: (start: Float, through: Float, end: Float)
    }

    private func cases(_ vm: DSPViewModel) -> [Case] {
        [
            Case(setting: .loudnessRef, commit: vm.setLoudnessRef, shown: { vm.loudness.refSPL }, values: (60, 70, 80)),
            Case(setting: .loudnessIntensity, commit: vm.setLoudnessIntensity, shown: { vm.loudness.intensity },
                 values: (50, 100, 150)),
            Case(setting: .crossfeedFreq, commit: vm.setCrossfeedFreq, shown: { vm.crossfeed.freq }, values: (600, 800, 1000)),
            Case(setting: .crossfeedFeed, commit: vm.setCrossfeedFeed, shown: { vm.crossfeed.feed }, values: (3, 5, 7)),
            Case(setting: .psybassCutoff, commit: vm.setPsybassCutoff, shown: { vm.psybass.cutoffHz }, values: (60, 90, 120)),
            Case(setting: .psybassHarmonics, commit: vm.setPsybassHarmonics, shown: { vm.psybass.harmonicsDB },
                 values: (-6, -3, 3)),
            Case(setting: .psybassDrive, commit: vm.setPsybassDrive, shown: { vm.psybass.driveDB }, values: (3, 6, 9)),
            Case(setting: .psybassCharacter, commit: vm.setPsybassCharacter, shown: { vm.psybass.characterPct },
                 values: (20, 40, 60)),
            Case(setting: .psybassOriginal, commit: vm.setPsybassOriginal, shown: { vm.psybass.originalDB },
                 values: (-20, -10, -5)),
            Case(setting: .subharmLow, commit: vm.setSubharmLow, shown: { vm.subharm.lowDB }, values: (-20, -10, 0)),
            Case(setting: .subharmHigh, commit: vm.setSubharmHigh, shown: { vm.subharm.highDB }, values: (-20, -10, 0)),
            Case(setting: .subharmTop, commit: vm.setSubharmTop, shown: { vm.subharm.topDB }, values: (-20, -10, 0)),
            Case(setting: .subharmBoost, commit: vm.setSubharmBoost, shown: { vm.subharm.boostDB }, values: (1, 2, 3)),
            Case(setting: .subharmDepth, commit: vm.setSubharmSelectDepth, shown: { vm.subharm.selectDepthPct },
                 values: (20, 40, 60)),
            Case(setting: .subharmHold, commit: vm.setSubharmSelectHold, shown: { vm.subharm.selectHoldMs },
                 values: (100, 200, 300)),
            Case(setting: .subharmCeiling, commit: vm.setSubharmCeiling, shown: { vm.subharm.ceilingDB },
                 values: (-20, -10, -5)),
            Case(setting: .levellerAmount, commit: vm.setLevellerAmount, shown: { vm.leveller.amount }, values: (20, 40, 60)),
            Case(setting: .levellerMaxGain, commit: vm.setLevellerMaxGain, shown: { vm.leveller.maxGainDB },
                 values: (5, 10, 15)),
            Case(setting: .levellerGate, commit: vm.setLevellerGate, shown: { vm.leveller.gateDB }, values: (-60, -50, -40)),
        ]
    }

    /// Every live setting is covered here, once, under its own registry path.
    func testSWS7EveryLiveSettingHasItsOwnPath() {
        let (_, _, vm) = demoVM()
        let covered = cases(vm).map(\.setting)
        XCTAssertEqual(covered.count, DSPViewModel.LiveFloat.allCases.count, "the tests cover every live setting")
        XCTAssertEqual(Set(covered), Set(DSPViewModel.LiveFloat.allCases))
        XCTAssertEqual(Set(DSPViewModel.LiveFloat.allCases.map(\.path)).count, DSPViewModel.LiveFloat.allCases.count,
                       "one path each")
        for s in DSPViewModel.LiveFloat.allCases {
            // The setters' commits find the same lane from their raw code.
            XCTAssertEqual(DSPViewModel.LiveFloat(request: s.rawRequest), s)
            XCTAssertEqual(DSPViewModel.typedFloatSet(s.rawRequest)?.lane, s.path)
        }
    }

    /// A drag on each effect's slider, as `ParameterRow` makes it: the drag
    /// mark on, live values in the setting's lane, the mark's end queued one
    /// main-loop turn (as `CoreUndo.drag` does) and the commit on release.
    /// One Undo goes back to where the drag began (not to a value it passed
    /// through), and one Redo to where it was released.
    func testSWS7ADragOnEachEffectIsOneUndoStepThroughTheCore() {
        let (core, t, vm) = demoVM()
        // -- as SW10's crossfeed test: a custom freq and feed show under Custom (3).
        vm.setCrossfeedPreset(3)
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.preset == 3 }, "Custom chosen")
        settle()
        let mark = CoreDragMark(mark: { on in
            if on { t.setDragging(true) } else { DispatchQueue.main.async { t.setDragging(false) } }
        })
        for c in cases(vm) {
            let name = c.setting.path
            c.commit(c.values.start)   // a known start, so the drag is a change
            flush(t)
            XCTAssertTrue(spinUntil(5) { c.shown() == c.values.start }, "\(name): start shown (\(c.shown()))")
            let before = entries(core, t, name).count

            mark.set(true)
            for step in 0...8 {
                let v = c.values.start + (c.values.through - c.values.start) * Float(step) / 8
                vm.sendFloatParamToDevice(c.setting, v)
                RunLoop.current.run(until: Date().addingTimeInterval(0.005))
            }
            mark.set(false)
            c.commit(c.values.end)
            flush(t)
            settle(0.1)
            flush(t)

            XCTAssertGreaterThan(entries(core, t, name).count, before, "\(name): the drag went to the typed setter")
            XCTAssertTrue(spinUntil(5) { c.shown() == c.values.end }, "\(name): released value shown (\(c.shown()))")
            XCTAssertTrue(spinUntil(5) { vm.undo.canUndo }, "\(name): Undo enabled")
            vm.undo.undo()
            XCTAssertTrue(spinUntil(5) { c.shown() == c.values.start },
                          "\(name): one Undo is the whole drag (\(c.shown()))")
            XCTAssertTrue(spinUntil(5) { vm.undo.canRedo }, "\(name): Redo enabled")
            vm.undo.redo()
            XCTAssertTrue(spinUntil(5) { c.shown() == c.values.end }, "\(name): one Redo is the whole drag (\(c.shown()))")
        }
        XCTAssertFalse(mark.isOn)
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no live send went out raw")
    }

    /// The opt-out (no core): the same live send is the raw float SET, and
    /// the device holds the value.  The GET codes are named here, in the
    /// test, so the views need none.
    func testSWS7WithoutACoreTheLiveSendIsTheRawSet() {
        let fake = FakeTransport(autoSelect: false)
        let vm = DSPViewModel(transport: fake)
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == fake.generation })
        settle()
        XCTAssertNil(vm.typedCore, "the opt-out has no core")
        let reads: [(DSPViewModel.LiveFloat, UInt8, Float)] = [
            (.loudnessRef, REQ_GET_LOUDNESS_REF, 72),
            (.crossfeedFeed, REQ_GET_CROSSFEED_FEED, 6),
            (.psybassHarmonics, REQ_GET_PSYBASS_HARMONICS, -4),
            (.subharmCeiling, REQ_GET_SUBHARM_CEILING, -15),
            (.levellerGate, REQ_GET_LEVELLER_GATE, -55),
        ]
        // The raw SET goes out with wValue 0 and wIndex 0; the fake answers
        // its GET twin under the same addressing.
        func held(_ get: UInt8) -> Float? {
            let d = fake.getControlRequest(request: get, value: 0, index: 0, length: 4)
            return d.flatMap { $0.count >= 4 ? $0.withUnsafeBytes { $0.load(as: Float.self) } : nil }
        }
        for (setting, get, value) in reads {
            vm.sendFloatParamToDevice(setting, value)
            XCTAssertTrue(spinUntil(5) { held(get) == value },
                          "\(setting.path) reached the device raw (\(String(describing: held(get))))")
        }
    }
}
