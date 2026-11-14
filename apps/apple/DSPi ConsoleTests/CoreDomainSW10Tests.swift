import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW10: the effects (loudness, crossfeed and its pair map, psychoacoustic
/// bass, subharmonic synth, tube, limiter, upmixer, leveller) through the
/// core's typed setters (Commands.swift, ExtensionCommands.swift), their
/// drag-time sends through SW9's write lane.  The edits go through the view
/// model as the views make them, and Undo and Redo step them back through
/// the core's journal.
final class CoreDomainSW10Tests: XCTestCase, CoreDomainTesting {

    /// `a`, or `b` when the setting already holds `a`: an edit that changes it.
    private func other<T: Equatable>(_ current: T, _ a: T, _ b: T) -> T { current == a ? b : a }

    // MARK: - Undo and Redo, one effect at a time

    func testSW10UndoAndRedoALoudnessEdit() {
        let (_, _, vm) = demoVM()
        XCTAssertFalse(vm.undo.canUndo, "a new connection has nothing to undo")
        let ref = vm.loudness.refSPL
        let to = other(ref, Float(70), 90)
        vm.setLoudnessRef(to)
        undoAndRedo(vm, undone: { vm.loudness.refSPL == ref }, redone: { vm.loudness.refSPL == to })

        let on = vm.loudnessEnabled
        vm.setLoudness(!on)
        undoAndRedo(vm, undone: { vm.loudnessEnabled == on }, redone: { vm.loudnessEnabled == !on })
    }

    func testSW10UndoAndRedoACrossfeedEdit() {
        let (_, _, vm) = demoVM()
        // -- WP SW12 -- fixup sw12 F1: since P1XF the core shows a named
        // preset's feed while one is chosen, so a custom feed edit shows only
        // under Custom (3), as the view's sliders are only enabled there.
        vm.setCrossfeedPreset(3)
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.preset == 3 }, "Custom chosen")
        settle()
        let feed = vm.crossfeed.feed
        let to = other(feed, Float(8), 12)
        vm.setCrossfeedFeed(to)
        undoAndRedo(vm, undone: { vm.crossfeed.feed == feed }, redone: { vm.crossfeed.feed == to })

        let itd = vm.crossfeed.itd
        vm.setCrossfeedITD(!itd)
        undoAndRedo(vm, undone: { vm.crossfeed.itd == itd }, redone: { vm.crossfeed.itd == !itd })
    }

    func testSW10UndoAndRedoAPsychoacousticBassEdit() {
        let (_, _, vm) = demoVM()
        let cutoff = vm.psybass.cutoffHz
        let to = other(cutoff, Float(120), 150)
        vm.setPsybassCutoff(to)
        undoAndRedo(vm, undone: { vm.psybass.cutoffHz == cutoff }, redone: { vm.psybass.cutoffHz == to })

        let mask = vm.psybass.outputMask
        vm.setPsybassOutputChannel(1, enabled: mask & 0b10 == 0)
        let edited = mask ^ 0b10
        undoAndRedo(vm, undone: { vm.psybass.outputMask == mask }, redone: { vm.psybass.outputMask == edited })
    }

    func testSW10UndoAndRedoASubharmEdit() {
        let (_, _, vm) = demoVM()
        let low = vm.subharm.lowDB
        let to = other(low, Float(-6), 3)
        vm.setSubharmLow(to)
        undoAndRedo(vm, undone: { vm.subharm.lowDB == low }, redone: { vm.subharm.lowDB == to })

        let mode = vm.subharm.selectMode
        let toMode = other(mode, SUBHARM_SELECT_PERCUSSIVE, SUBHARM_SELECT_SUSTAINED)
        vm.setSubharmSelectMode(toMode)
        undoAndRedo(vm, undone: { vm.subharm.selectMode == mode }, redone: { vm.subharm.selectMode == toMode })
    }

    func testSW10UndoAndRedoATubeEdit() {
        let (_, _, vm) = demoVM()
        let drive = vm.tube.driveDB
        let to = other(drive, Float(6), 12)
        vm.setTubeDrive(to)
        undoAndRedo(vm, undone: { vm.tube.driveDB == drive }, redone: { vm.tube.driveDB == to })
    }

    /// A tube type loads its row into the four character knobs.  Undo puts
    /// the type and the knobs back: the core journals the row as the type's
    /// companions.
    func testSW10UndoOfATubeTypeRestoresItsCharacter() throws {
        let (_, _, vm) = demoVM()
        let (type, bias, sag) = (vm.tube.type, vm.tube.biasPct, vm.tube.sagPct)
        let toType = (1...TUBE_TYPE_MAX).first { t in
            guard t != type, t < TUBE_TYPE_ROWS.count, let row = TUBE_TYPE_ROWS[t] else { return false }
            return row.biasPct != bias || row.sagPct != sag
        }
        let t = try XCTUnwrap(toType, "a type whose row differs from the knobs")
        let row = try XCTUnwrap(TUBE_TYPE_ROWS[t])
        vm.setTubeType(t)
        XCTAssertEqual(vm.tube.biasPct, row.biasPct, "the row is shown at once")
        undoAndRedo(vm,
                    undone: { vm.tube.type == type && vm.tube.biasPct == bias && vm.tube.sagPct == sag },
                    redone: { vm.tube.type == t && vm.tube.biasPct == row.biasPct && vm.tube.sagPct == row.sagPct })
    }

    /// The limiter's threshold.  The enable is an audible change: the core
    /// records it and Undo steps over it, as in Qt.
    func testSW10UndoAndRedoALimiterEdit() {
        let (core, t, vm) = demoVM()
        let threshold = vm.limiter.outputs[1].thresholdDB
        let to = other(threshold, Float(-12), -6)
        vm.sendLimiterParamToDevice(output: 1, LIMITER_PARAM_THRESHOLD_DB, to)   // a drag, then its commit
        vm.setLimiterThreshold(output: 1, to)
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no limiter write went out raw")
        undoAndRedo(vm, undone: { vm.limiter.outputs[1].thresholdDB == threshold },
                    redone: { vm.limiter.outputs[1].thresholdDB == to })
    }

    /// A linked group is moved by the core (Swift's LimiterGang is not what
    /// the view shows through a core): a threshold on one member reaches the
    /// other, and one Undo puts both back.
    func testSW10ALimiterGroupMovesTogetherAndUndoesTogether() {
        let (core, t, vm) = demoVM()
        vm.setLimiterLinkGroup(output: 0, 2)
        vm.setLimiterLinkGroup(output: 1, 2)
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[1].linkGroup == 2 })
        let before = vm.limiter.outputs[0].thresholdDB
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[1].thresholdDB == before }, "the joiner adopts the group's")
        let to = other(before, Float(-15), -9)
        vm.setLimiterThreshold(output: 0, to)
        XCTAssertTrue(spinUntil(5) { vm.limiter.outputs[0].thresholdDB == to && vm.limiter.outputs[1].thresholdDB == to })
        XCTAssertEqual(entries(core, t, "limit.threshold").count, 1, "journalled")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) {
            vm.limiter.outputs[0].thresholdDB == before && vm.limiter.outputs[1].thresholdDB == before
        }, "both members back: \(vm.limiter.outputs[0].thresholdDB), \(vm.limiter.outputs[1].thresholdDB)")
    }

    func testSW10UndoAndRedoAnUpmixerEdit() {
        let (_, _, vm) = demoVM()
        let strength = vm.upmix.strengthPct
        let to = other(strength, Float(40), 60)
        vm.setUpmixStrength(to)
        undoAndRedo(vm, undone: { vm.upmix.strengthPct == strength }, redone: { vm.upmix.strengthPct == to })

        let mode = vm.upmix.surroundMode
        let toMode = other(mode, 1, 2)
        vm.setUpmixSurroundMode(toMode)
        undoAndRedo(vm, undone: { vm.upmix.surroundMode == mode }, redone: { vm.upmix.surroundMode == toMode })
    }

    func testSW10UndoAndRedoALevellerEdit() {
        let (_, _, vm) = demoVM()
        let amount = vm.leveller.amount
        let to = other(amount, Float(30), 70)
        vm.setLevellerAmount(to)
        undoAndRedo(vm, undone: { vm.leveller.amount == amount }, redone: { vm.leveller.amount == to })

        let speed = vm.leveller.speed
        let toSpeed = other(speed, 0, 2)
        vm.setLevellerSpeed(toSpeed)
        undoAndRedo(vm, undone: { vm.leveller.speed == speed }, redone: { vm.leveller.speed == toSpeed })
    }

    /// An effect slider's drag (ParameterRow's live closure, then its commit
    /// on release): many lane writes, one Undo step.
    func testSW10AnEffectSliderDragIsOneUndoStep() {
        let (core, t, vm) = demoVM()
        let before = vm.psybass.harmonicsDB
        t.setDragging(true)
        for step in 1...10 {
            vm.sendFloatParamToDevice(REQ_SET_PSYBASS_HARMONICS, -12 + Float(step))
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        vm.setPsybassHarmonics(-2)
        t.setDragging(false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no drag write went out raw")
        XCTAssertFalse(entries(core, t, "bass.harmonics").isEmpty, "the drag went to the typed setter")
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.psybass.harmonicsDB == before }, "one step: \(vm.psybass.harmonicsDB)")
        XCTAssertTrue(spinUntil(5) { !vm.undo.canUndo })
    }

    // MARK: - No raw writes in this domain

    private static func float(_ fake: FakeTransport, _ request: UInt8, _ value: UInt16 = 0) -> Float? {
        guard let d = fake.getControlRequest(request: request, value: value, index: HostBridge.vendorInterface,
                                             length: 4), d.count >= 4 else {
            return nil
        }
        return d.withUnsafeBytes { $0.load(as: Float.self) }
    }

    private static func byte(_ fake: FakeTransport, _ request: UInt8) -> UInt8? {
        fake.getControlRequest(request: request, value: 0, index: HostBridge.vendorInterface, length: 1)?.first
    }

    /// Every setter and live send of the effects goes to a typed core call:
    /// the journal holds no raw entry, each lands under its own path, and
    /// the device (Swift's FakeTransport, through the core) holds the values.
    func testSW10EveryEffectWriteIsTypedAndReachesTheDevice() {
        // -- fixup sw10 -- the fake answers the core's effect probes from its
        // image, as firmware with the effects does: no priming, no reconnect.
        let (fake, core, t, vm) = hostVM()

        vm.setLoudness(true)
        vm.setLoudnessRef(75)
        vm.sendFloatParamToDevice(REQ_SET_LOUDNESS_INTENSITY, 120)
        vm.setLoudnessIntensity(130)
        vm.setLoudnessMask(0b11)
        vm.setCrossfeed(true)
        vm.setCrossfeedPreset(3)
        vm.sendFloatParamToDevice(REQ_SET_CROSSFEED_FREQ, 800)
        vm.setCrossfeedFreq(900)
        vm.setCrossfeedFeed(7)
        let itd = !vm.crossfeed.itd   // a change, so it is journalled
        vm.setCrossfeedITD(itd)
        let pairs = vm.crossfeed.outputMask ^ 0b1
        vm.setCrossfeedMask(pairs)
        vm.setPsybass(true)
        vm.setPsybassCutoff(110)
        vm.setPsybassDrive(9)
        vm.setPsybassMask(0b101)
        vm.setSubharm(true)
        vm.setSubharmHigh(-4)
        vm.sendFloatParamToDevice(REQ_SET_SUBHARM_CEILING, -10)
        vm.setSubharmCeiling(-12)
        let linked = !vm.subharm.linkPairs
        vm.setSubharmLinkPairs(linked)
        vm.setSubharmSolo(true)
        vm.setSubharmMask(0b110)
        vm.setTube(true)
        vm.sendTubeParamToDevice(TUBE_PARAM_MIX_PCT, 40, TUBE_MIX_MIN, TUBE_MIX_MAX)
        vm.setTubeMix(50)
        vm.setUpmixEnabled(true)
        vm.sendUpmixParamToDevice(UPMIX_PARAM_ATTACK, 30)
        vm.setUpmixAttack(35)
        vm.setLeveller(true)
        vm.setLevellerGate(-50)
        let lookahead = !vm.leveller.lookahead
        vm.setLevellerLookahead(lookahead)
        vm.setLevellerMasks(detector: 0b11, apply: 0b01)
        // -- fixup sw10 -- the limiter on the fake too (its probe now answers).
        vm.sendLimiterParamToDevice(output: 1, LIMITER_PARAM_THRESHOLD_DB, -14)
        vm.setLimiterThreshold(output: 1, -12)
        vm.setLimiterRelease(output: 1, 250)
        flush(t)
        settle()

        XCTAssertTrue(entries(core, t, "raw").isEmpty, "no effect write went out raw")
        for path in ["loud.on", "loud.ref", "loud.intensity", "loud.mask", "cf.on", "cf.preset", "cf.freq", "cf.feed",
                     "cf.itd", "cf.outputs", "bass.on", "bass.cutoff", "bass.drive", "bass.mask", "sub.on", "sub.high",
                     "sub.ceiling", "sub.link", "sub.solo", "sub.mask", "tube.on", "tube.mix", "up.on", "up.attack", "lev.on", "lev.gate", "lev.lookahead", "lev.masks",
                     "limit.threshold", "limit.release"] {
            XCTAssertFalse(entries(core, t, path).isEmpty, "\(path) went through its typed setter")
        }

        // What the device holds, read straight from the fake.
        XCTAssertEqual(Self.byte(fake, REQ_GET_LOUDNESS), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_LOUDNESS_REF), 75)
        XCTAssertEqual(Self.float(fake, REQ_GET_LOUDNESS_INTENSITY), 130)
        XCTAssertEqual(Self.byte(fake, REQ_GET_CROSSFEED_PRESET), 3)
        XCTAssertEqual(Self.float(fake, REQ_GET_CROSSFEED_FREQ), 900)
        XCTAssertEqual(Self.float(fake, REQ_GET_CROSSFEED_FEED), 7)
        XCTAssertEqual(Self.byte(fake, REQ_GET_CROSSFEED_ITD), itd ? 1 : 0)
        XCTAssertEqual(Self.byte(fake, REQ_GET_PSYBASS), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_PSYBASS_CUTOFF), 110)
        XCTAssertEqual(Self.float(fake, REQ_GET_PSYBASS_DRIVE), 9)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_SUBHARM_HIGH), -4)
        XCTAssertEqual(Self.float(fake, REQ_GET_SUBHARM_CEILING), -12)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM_LINK), linked ? 1 : 0)
        XCTAssertEqual(Self.byte(fake, REQ_GET_SUBHARM_SOLO), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_TUBE_PARAM, TUBE_PARAM_MIX_PCT), 50)
        XCTAssertEqual(Self.byte(fake, REQ_GET_LEVELLER), 1)
        XCTAssertEqual(Self.float(fake, REQ_GET_LEVELLER_GATE), -50)
        XCTAssertEqual(Self.byte(fake, REQ_GET_LEVELLER_LOOKAHEAD), lookahead ? 1 : 0)
        XCTAssertEqual(Self.float(fake, REQ_LIMITER, (1 << 8) | UInt16(LIMITER_PARAM_THRESHOLD_DB)), -12)
        XCTAssertEqual(Self.float(fake, REQ_LIMITER, (1 << 8) | UInt16(LIMITER_PARAM_RELEASE_MS)), 250)

        // And the core's mirror, which the view model shows.
        let s = t.readState()
        XCTAssertEqual(s.upmix[Int(DspiConstants.UPMIX_PARAM_ATTACK)], 35)
        XCTAssertEqual(s.upmix[Int(DspiConstants.UPMIX_PARAM_ENABLED)], 1)
        XCTAssertEqual(s.subharm[Int(DspiConstants.SUBHARM_PARAM_MASK)], 6)
        XCTAssertEqual(s.psybassOutputMask, 0b101)
        XCTAssertEqual(s.loudnessOutputMask, 0b11)
        XCTAssertEqual(s.levellerDetectorMask, 0b11)
        XCTAssertEqual(s.levellerApplyMask, 0b01)
    }

    /// Every float SET a view sends live has its typed setter (none of them
    /// falls back to raw through a core).
    func testSW10EveryLiveFloatRequestHasATypedSetter() {
        let live: [UInt8] = [REQ_SET_LOUDNESS_REF, REQ_SET_LOUDNESS_INTENSITY, REQ_SET_CROSSFEED_FREQ,
                             REQ_SET_CROSSFEED_FEED, REQ_SET_PSYBASS_CUTOFF, REQ_SET_PSYBASS_HARMONICS,
                             REQ_SET_PSYBASS_DRIVE, REQ_SET_PSYBASS_CHARACTER, REQ_SET_PSYBASS_ORIGINAL,
                             REQ_SET_SUBHARM_LOW, REQ_SET_SUBHARM_HIGH, REQ_SET_SUBHARM_TOP, REQ_SET_SUBHARM_BOOST,
                             REQ_SET_SUBHARM_DEPTH, REQ_SET_SUBHARM_HOLD, REQ_SET_SUBHARM_CEILING,
                             REQ_SET_LEVELLER_AMOUNT, REQ_SET_LEVELLER_MAXGAIN, REQ_SET_LEVELLER_GATE]
        for request in live {
            XCTAssertNotNil(DSPViewModel.typedFloatSet(request), String(format: "0x%02X", request))
        }
        XCTAssertEqual(Set(live.compactMap { DSPViewModel.typedFloatSet($0)?.lane }).count, live.count,
                       "one lane key each")
    }

    /// The crossfeed pair map: written and read back by the core (it checks
    /// the map, sends it normalized and reads it back), and an invalid one
    /// refused before anything is sent.
    func testSW10ThePairMapGoesThroughTheCore() {
        let (_, core, t, vm) = hostVM()   // the fake answers like the bofr fork
        DispatchQueue.global().sync { vm.probeExtensions(force: true) }
        XCTAssertTrue(spinUntil(5) { vm.crossfeedPairMapSupported }, "the fake lists the pair map")
        let before = vm.crossfeed.pairMap
        XCTAssertNotEqual(before, .oneDACPerEar)
        XCTAssertTrue(vm.setCrossfeedPairMap(.oneDACPerEar))
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.pairMap == .oneDACPerEar }, "the core's read-back is shown")
        XCTAssertTrue(entries(core, t, "raw").isEmpty, "the map did not go raw")
        XCTAssertEqual(DispatchQueue.global().sync { vm.readCrossfeedPairMap() }, .oneDACPerEar)
        XCTAssertFalse(vm.setCrossfeedPairMap(CrossfeedPairMap(bytes: [0x11, 0, 0, 0])), "invalid: not sent")

        // Live routing, as a crossfeed setting: Undo puts the pairing back.
        undoAndRedo(vm, undone: { vm.crossfeed.pairMap == before }, redone: { vm.crossfeed.pairMap == .oneDACPerEar })
    }

    // MARK: - Reads and polls through the core

    /// The subharm meter and solo on the core's demo device, through the
    /// core's calls (the poll timer's path).
    func testSW10SubharmMeterAndSoloThroughTheCore() {
        let (_, t, vm) = demoVM()
        vm.subharm.meter.levels = []
        DispatchQueue.global().async { vm.fetchSubharmMeter() }
        XCTAssertTrue(spinUntil(5) { vm.subharm.meter.levels.count == vm.numOutputChannels },
                      "one level per output: \(vm.subharm.meter.levels.count)")

        vm.setSubharmSolo(true)
        flush(t)
        vm.subharm.solo = false
        DispatchQueue.global().async { vm.fetchSubharmSolo() }
        XCTAssertTrue(spinUntil(5) { vm.subharm.solo }, "the core's copy")
        vm.setSubharmSolo(false)
    }
    // MARK: - fixup sw10

    private static func floatValue(_ v: ParamValue) -> Float? {
        if case .float(let f) = v { return f }
        return nil
    }

    /// F1: a lane value issued after a commit of the same effect setting is
    /// sent after it (the review's probes P1-P3), for every float setting a
    /// drag sends live and for the tube, the limiter and the upmixer: the
    /// commit supersedes the value still queued before it, so the device
    /// ends on the newest value issued and the superseded one is never sent.
    func testSW10ALaneValueAfterACommitIsNotOvertaken() {
        let (core, t, vm) = demoVM()
        typealias Case = (path: String, values: (Float, Float, Float), lane: (Float) -> Void, commit: (Float) -> Void)
        func float(_ path: String, _ request: UInt8, _ v: (Float, Float, Float), _ commit: @escaping (Float) -> Void) -> Case {
            (path, v, { vm.sendFloatParamToDevice(request, $0) }, commit)
        }
        let cases: [Case] = [
            float("loud.ref", REQ_SET_LOUDNESS_REF, (60, 70, 80), vm.setLoudnessRef),
            float("loud.intensity", REQ_SET_LOUDNESS_INTENSITY, (50, 100, 150), vm.setLoudnessIntensity),
            float("cf.freq", REQ_SET_CROSSFEED_FREQ, (600, 800, 1000), vm.setCrossfeedFreq),
            float("cf.feed", REQ_SET_CROSSFEED_FEED, (3, 5, 7), vm.setCrossfeedFeed),
            float("bass.cutoff", REQ_SET_PSYBASS_CUTOFF, (60, 90, 120), vm.setPsybassCutoff),
            float("bass.harmonics", REQ_SET_PSYBASS_HARMONICS, (-6, -3, 3), vm.setPsybassHarmonics),
            float("bass.drive", REQ_SET_PSYBASS_DRIVE, (3, 6, 9), vm.setPsybassDrive),
            float("bass.character", REQ_SET_PSYBASS_CHARACTER, (20, 40, 60), vm.setPsybassCharacter),
            float("bass.original", REQ_SET_PSYBASS_ORIGINAL, (-20, -10, -5), vm.setPsybassOriginal),
            float("sub.low", REQ_SET_SUBHARM_LOW, (-20, -10, 0), vm.setSubharmLow),
            float("sub.high", REQ_SET_SUBHARM_HIGH, (-20, -10, 0), vm.setSubharmHigh),
            float("sub.top", REQ_SET_SUBHARM_TOP, (-20, -10, 0), vm.setSubharmTop),
            float("sub.boost", REQ_SET_SUBHARM_BOOST, (1, 2, 3), vm.setSubharmBoost),
            float("sub.depth", REQ_SET_SUBHARM_DEPTH, (20, 40, 60), vm.setSubharmSelectDepth),
            float("sub.hold", REQ_SET_SUBHARM_HOLD, (100, 200, 300), vm.setSubharmSelectHold),
            float("sub.ceiling", REQ_SET_SUBHARM_CEILING, (-20, -10, -5), vm.setSubharmCeiling),
            float("lev.amount", REQ_SET_LEVELLER_AMOUNT, (20, 40, 60), vm.setLevellerAmount),
            float("lev.maxgain", REQ_SET_LEVELLER_MAXGAIN, (5, 10, 15), vm.setLevellerMaxGain),
            float("lev.gate", REQ_SET_LEVELLER_GATE, (-60, -50, -40), vm.setLevellerGate),
            ("tube.drive", (3, 6, 9), { vm.sendTubeParamToDevice(TUBE_PARAM_DRIVE_DB, $0, TUBE_DRIVE_MIN, TUBE_DRIVE_MAX) },
             vm.setTubeDrive),
            ("limit.threshold", (-20, -15, -10), { vm.sendLimiterParamToDevice(output: 1, LIMITER_PARAM_THRESHOLD_DB, $0) },
             { vm.setLimiterThreshold(output: 1, $0) }),
            ("up.attack", (25, 30, 35), { vm.sendUpmixParamToDevice(UPMIX_PARAM_ATTACK, $0) }, vm.setUpmixAttack),
        ]
        XCTAssertEqual(cases.count, 22, "the 19 live float settings, the tube, the limiter and the upmixer")
        for c in cases {
            c.commit(c.values.0)   // a known start, so the commit below is a change
            flush(t)
            let busy = DispatchSemaphore(value: 0)
            t.coreQueue.async { busy.wait() }
            c.lane(c.values.0)     // a lane value, queued
            c.commit(c.values.1)   // the commit: it supersedes the queued value
            c.lane(c.values.2)     // newer than the commit
            busy.signal()
            flush(t)
            let sent = entries(core, t, c.path).compactMap { Self.floatValue($0.after) }   // newest first
            XCTAssertEqual(Array(sent.prefix(2)), [c.values.2, c.values.1],
                           "\(c.path): the commit, then the newer value; the superseded one never sent")
        }
        XCTAssertEqual(t.readState().loudnessRefSpl, 80, "the newest value issued is the device's")
    }

    /// The fake's limiter as the firmware has it: the status block answers
    /// (the core's probe), an edit reaches the whole link group, a joiner
    /// adopts the group's settings, and every output takes output 0xFF.
    func testSW10TheFakesLimiterGangsAsTheFirmware() throws {
        let fake = FakeTransport(autoSelect: false)
        fake.selectDevice(FakeTransport.demoDevice)
        func get(_ o: UInt8, _ p: UInt8) -> Float? {
            fake.getControlRequest(request: REQ_LIMITER, value: (UInt16(o) << 8) | UInt16(p), index: 0, length: 4)?
                .withUnsafeBytes { $0.loadUnaligned(as: Float.self) }
        }
        func set(_ o: UInt8, _ p: UInt8, _ v: Float) {
            fake.sendControlRequest(request: REQ_LIMITER, value: (UInt16(o) << 8) | UInt16(p), index: 0,
                                    data: withUnsafeBytes(of: v) { Data($0) })
        }
        let status = try XCTUnwrap(fake.getControlRequest(request: REQ_LIMITER, value: UInt16(LIMITER_GET_STATUS),
                                                          index: 0, length: 4))
        XCTAssertEqual(status.count, 4)
        let outputs = status[3]
        XCTAssertGreaterThan(outputs, 1)
        XCTAssertNil(get(outputs, LIMITER_PARAM_THRESHOLD_DB), "no such output: STALL")
        set(0, LIMITER_PARAM_THRESHOLD_DB, -6)
        set(1, LIMITER_PARAM_THRESHOLD_DB, -3)
        set(0, LIMITER_PARAM_LINK_GROUP, 2)
        set(1, LIMITER_PARAM_LINK_GROUP, 2)
        XCTAssertEqual(get(1, LIMITER_PARAM_THRESHOLD_DB), -6, "the joiner adopts the group's")
        set(1, LIMITER_PARAM_THRESHOLD_DB, -40)
        XCTAssertEqual(get(0, LIMITER_PARAM_THRESHOLD_DB), LIMITER_THRESHOLD_MIN, "clamped, and the group moves")
        set(LIMITER_ALL_OUTPUTS, LIMITER_PARAM_ENABLED, 1)
        XCTAssertEqual((0..<outputs).map { get($0, LIMITER_PARAM_ENABLED) }, Array(repeating: 1, count: Int(outputs)))
        XCTAssertEqual(fake.getControlRequest(request: REQ_LIMITER, value: UInt16(LIMITER_GET_STATUS),
                                              index: 0, length: 4)?.first, 1, "engaged")
    }

    /// A view model on Swift's fake under the core, `inner` between them, and
    /// a monitor on the transport running the view model's own
    /// BULK_INVALIDATED hook, as the app's shared monitor does.
    private func refusalVM(_ inner: (FakeTransport) -> any DeviceTransport)
        -> (FakeTransport, CoreBackedTransport, DSPViewModel, InterruptMonitor) {
        let fake = FakeTransport(autoSelect: false)
        let t = CoreBackedTransport(link: .host(inner(fake), .fake), core: DspiCore())
        let vm = DSPViewModel(transport: t)
        let monitor = InterruptMonitor(transport: t)
        monitor.onBulkInvalidated = AppState.shared.interruptMonitor.onBulkInvalidated   // this view model's
        monitor.start()
        fake.selectDevice(FakeTransport.demoDevice)
        XCTAssertTrue(spinUntil(30) { vm.isDeviceConnected && vm.fullReadGeneration == t.generation },
                      "the connect-time read finishes")
        settle()
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
        return (fake, t, vm, monitor)
    }

    /// F2: a device whose psychoacoustic-bass probe failed (the core has no
    /// such feature) while the view shows the controls: the core refuses the
    /// edit as Unsupported, and the view puts the core's value back at once
    /// and says why.
    func testSW10AnUnsupportedEffectWriteIsPutBackAndSaysWhy() {
        let (fake, t, vm, monitor) = refusalVM { StallingTransport($0, stalls: [REQ_GET_PSYBASS]) }
        defer { monitor.stop() }
        let before = vm.psybass.cutoffHz
        let to = other(before, Float(120), 150)
        vm.setPsybassCutoff(to)
        XCTAssertEqual(vm.psybass.cutoffHz, to, "shown at once, as the slider does")
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message == RefusedWrite.unsupportedMessage },
                      "the refusal is shown: \(DeviceLock.notice.message ?? "none")")
        XCTAssertTrue(spinUntil(3) { vm.psybass.cutoffHz == before }, "the core's value is back: \(vm.psybass.cutoffHz)")
        settle(CoreMirror.defaultPendingWindow + 0.3)
        XCTAssertEqual(vm.psybass.cutoffHz, before, "and stays")
        flush(t)
        XCTAssertNotEqual(fake.getControlRequest(request: REQ_GET_PSYBASS_CUTOFF, value: 0,
                                                 index: HostBridge.vendorInterface, length: 4)?
                            .withUnsafeBytes { $0.loadUnaligned(as: Float.self) }, to, "nothing reached the device")
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
    }

    /// F2: a typed write that fails in transfer (the core's `Transfer`) is
    /// put back to the core's value at once, with its wording.
    func testSW10ATypedWriteThatFailsInTransferIsPutBackAndSaysWhy() {
        let (_, t, vm, monitor) = refusalVM { $0 }
        defer { monitor.stop() }
        let before = vm.loudness.refSPL
        let to = other(before, Float(70), 90)
        vm.loudness.refSPL = to   // what the setter shows at once
        t.commit(lane: "loud.ref") { _ in throw DspiError.Transfer(msg: "test: the SET timed out") }
        XCTAssertTrue(spinUntil(3) { DeviceLock.notice.message == RefusedWrite.transferMessage },
                      "the failure is shown: \(DeviceLock.notice.message ?? "none")")
        XCTAssertTrue(spinUntil(3) { vm.loudness.refSPL == before }, "the core's value is back: \(vm.loudness.refSPL)")
        XCTAssertTrue(spinUntil(8) { DeviceLock.notice.message == nil })
    }
}

/// -- fixup sw10 -- STALLs the given GETs, as firmware without the feature.
final class StallingTransport: ForwardingTransport {
    private let stalls: Set<UInt8>
    init(_ inner: any DeviceTransport, stalls: Set<UInt8>) {
        self.stalls = stalls
        super.init(inner)
    }
    override func getControlResult(request: UInt8, value: UInt16, index: UInt16, length: UInt16) -> Result<Data, DSPi_Console.LinkStatus> {
        stalls.contains(request) ? .failure(.error) : super.getControlResult(request: request, value: value, index: index, length: length)
    }
}
