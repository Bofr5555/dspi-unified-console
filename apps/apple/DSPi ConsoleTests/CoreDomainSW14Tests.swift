import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW14: the analysis and diagnostics domain through the core, on the
/// core's demo device (which has the analyser and the signal generator,
/// CG22): the meters and the sample rate, the signal generator, the
/// analyser (the core's worker and its following state, CG5), the Stats
/// window and the Interrupt Monitor's log.  The signal generator is started
/// only here, on the demo device.
final class CoreDomainSW14Tests: XCTestCase, CoreDomainTesting {

    /// `body` off main, as the views call the setters that wait.
    private func offMain<T>(_ body: () -> T) -> T { DispatchQueue.global().sync(execute: body) }

    /// Every request this domain has on the wire, SET and GET.
    private static let domainRequests: Set<UInt8> = [
        REQ_RTA_SET_CONFIG, REQ_RTA_GET_CONFIG, REQ_RTA_GET_CAPS, REQ_RTA_GET_BANDS, REQ_RTA_GET_BINS,
        REQ_RTA_GET_STATUS, REQ_RTA_CONTROL, REQ_RTA_GET_BANDS_ALL,
        REQ_SIGGEN_GET_CAPS, REQ_SIGGEN_SET_CONFIG, REQ_SIGGEN_GET_CONFIG, REQ_SIGGEN_CONTROL, REQ_SIGGEN_GET_STATUS,
        REQ_GET_STATUS, REQ_CLEAR_CLIPS, REQ_RESET_BUFFER_STATS, REQ_GET_BUFFER_STATS, REQ_GET_I2S_SLAVE_STATUS,
        REQ_GET_ADAT_STATUS, REQ_GET_LG_SOUND_SYNC_STATUS, REQ_GET_SPDIF_RX_STATUS, REQ_GET_SPDIF_RX_CH_STATUS,
        REQ_GET_SPDIF_RX_PIN, REQ_GET_SERIAL, REQ_OUTPUT_LATENCY, REQ_GET_INPUT_SOURCE, REQ_GET_PLATFORM,
    ]

    private func domainRaw(_ t: CoreBackedTransport) -> [String] {
        t.takeRawOpcodesForTest().filter { Self.domainRequests.contains($0) }.map { String(format: "0x%02X", $0) }
    }

    /// The demo device with its analyser probed by the core's worker.
    private func rtaVM() -> (DspiCore, CoreBackedTransport, DSPViewModel) {
        let (core, t, vm) = demoVM()
        XCTAssertTrue(spinUntil(10) { vm.rta.supported }, "the core's worker probed the analyser")
        return (core, t, vm)
    }

    private func coreSiggenState(_ core: DspiCore, _ t: CoreBackedTransport) -> UInt8? {
        t.coreQueue.sync { try? core.siggenGetStatus().state }
    }

    // MARK: - The meters

    /// The meter poll and the sample rate are the core's reads.
    func testSW14MetersAndSampleRateThroughTheCore() {
        let (core, t, vm) = demoVM()
        _ = t.takeRawOpcodesForTest()
        vm.fetchStatus()
        vm.fetchSampleRate()
        settle()
        let rate = t.coreQueue.sync { try? core.fetchStat(which: DspiConstants.STAT_SAMPLE_RATE) }
        XCTAssertNotNil(rate)
        XCTAssertTrue(spinUntil(5) { vm.telemetry.sampleRateHz == rate }, "the core's rate is shown")
        let status = t.coreQueue.sync { try? core.fetchStatus() }
        XCTAssertNotNil(status)
        XCTAssertTrue(spinUntil(5) { vm.activeInputChannels == max(BASE_MATRIX_INPUTS, Int(status?.activeInputChannels ?? 0)) })
        vm.clearClips()
        flush(t)
        XCTAssertTrue(spinUntil(5) { vm.meters.status.clipLatched == 0 }, "no clip held after a clear")
        XCTAssertEqual(domainRaw(t), [])
    }

    // MARK: - The signal generator

    /// The caps, the applied config and the status come from the core at
    /// the connect, and nothing starts the generator by itself.
    func testSW14SignalGeneratorCapsAndNeverStartsByItself() {
        let (core, t, vm) = demoVM()
        XCTAssertTrue(spinUntil(10) { vm.siggenSupported }, "the connect-time read fetched the generator's caps")
        let caps = t.coreQueue.sync { try? core.siggenFetchCaps() }
        XCTAssertEqual(vm.siggenCaps.typeCount, caps?.typeCount)
        XCTAssertEqual(vm.siggenCaps.validChannelMask, caps?.validMask)
        XCTAssertEqual(vm.siggenCaps.multitoneMax, caps?.multitoneMax)
        XCTAssertEqual(vm.siggenTypeDescs.count, Int(caps?.typeCount ?? 0), "one descriptor per type")
        XCTAssertEqual(vm.siggenTypeDescs.first?.name, caps?.types.first?.name)
        XCTAssertEqual(vm.siggenTypeDescs.first?.params.count, 4)
        XCTAssertFalse(vm.siggenStatus.isRunning)
        settle(0.5)
        XCTAssertEqual(coreSiggenState(core, t), SIGGEN_STATE_IDLE, "never started without a Start")
    }

    /// Start, live edit, stop and the identify tone on the demo device, all
    /// typed; the core journals them as history only, so Undo steps over
    /// them and never starts the generator again.
    func testSW14SignalGeneratorThroughTheCoreOnTheDemoDevice() throws {
        let (core, t, vm) = demoVM()
        XCTAssertTrue(spinUntil(10) { vm.siggenSupported })
        _ = t.takeRawOpcodesForTest()
        let volume = vm.masterVolumeDB
        vm.setMasterVolume(volume - 6)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })

        var cfg = DSPi_Console.SiggenConfig()
        cfg.signalType = SIGGEN_SINE
        cfg.channelMask = 1
        cfg.levelDB = -40
        cfg.p1 = 1000
        offMain { vm.siggenStart(with: cfg) }
        XCTAssertTrue(spinUntil(5) { vm.siggenStatus.isRunning }, "running on the demo device")
        let applied = t.coreQueue.sync { try? core.siggenGetConfig() }
        XCTAssertEqual(applied?.signalType, SIGGEN_SINE)
        XCTAssertEqual(applied?.channelMask, 1)
        XCTAssertEqual(applied?.p.first, 1000)

        cfg.p1 = 2000
        vm.siggenSetConfig(cfg)
        flush(t)
        XCTAssertEqual(t.coreQueue.sync { try? core.siggenGetConfig() }?.p.first, 2000, "a live edit is staged")

        offMain { vm.siggenStop(immediate: true) }
        XCTAssertTrue(spinUntil(5) { !vm.siggenStatus.isRunning }, "stopped")
        XCTAssertFalse(offMain { vm.siggenControl(0x7F) }, "an unknown action is refused, as the stall was")

        XCTAssertFalse(try XCTUnwrap(entries(core, t, "sig.config").last).undoable, "history only")
        XCTAssertFalse(try XCTUnwrap(entries(core, t, "sig.control").last).undoable, "history only")
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == volume }, "the volume change is undone")
        XCTAssertNotNil(vm.undo.notice, "the step over is named")
        settle(0.5)
        XCTAssertEqual(coreSiggenState(core, t), SIGGEN_STATE_IDLE, "Undo never starts the generator")

        // The identify tone: a channel-ID burst on one output, then idle.
        vm.identifyOutput(0)
        XCTAssertTrue(spinUntil(5) { (t.coreQueue.sync { try? core.siggenGetConfig() })?.signalType == SIGGEN_CHANNEL_ID })
        offMain { vm.siggenStop(immediate: true) }
        XCTAssertTrue(spinUntil(5) { self.coreSiggenState(core, t) == SIGGEN_STATE_IDLE })
        XCTAssertEqual(domainRaw(t), [])
    }

    /// The conversions both ways keep every field.
    func testSW14SiggenRecordsConvert() {
        var cfg = DSPi_Console.SiggenConfig()
        cfg.signalType = SIGGEN_SINE
        cfg.channelMask = 0x0105
        cfg.invertMask = 0x0004
        cfg.flags = 1
        cfg.levelDB = -12.5
        cfg.durationMS = 250
        cfg.repeatCount = 3
        cfg.gapMS = 40
        cfg.p1 = 1; cfg.p2 = 2; cfg.p3 = 3; cfg.p4 = 4
        XCTAssertEqual(DSPi_Console.SiggenConfig(core: cfg.core), cfg)
        XCTAssertEqual(cfg.core.p, [1, 2, 3, 4])
        let status = DSPi_Console.SiggenStatus(core: dspi_core.SiggenStatus(state: SIGGEN_STATE_RUN, signalType: 2,
                                                              activeChannel: 0xFF, elapsedMs: 1234, cyclesDone: 5,
                                                              stopReason: 0, currentFreq: 440))
        XCTAssertTrue(status.isRunning)
        XCTAssertEqual(status.elapsedMS, 1234)
        XCTAssertEqual(status.currentFreq, 440)
    }

    // MARK: - The analyser

    /// The core's worker probes the analyser; a view's request goes to the
    /// core, whose frames, bins and status reach the renderers' snapshot;
    /// releasing the last view stops it.  Nothing of the analyser goes out
    /// raw (the engine's poll does nothing through a core).
    func testSW14AnalyserThroughTheCore() throws {
        let (core, t, vm) = rtaVM()
        let caps = t.coreQueue.sync { core.rtaGetCaps() }
        XCTAssertEqual(vm.rta.caps.maxBands, caps.maxBands)
        XCTAssertEqual(vm.rta.bandCentresHz.count, Int(caps.maxBands))
        XCTAssertEqual(vm.rta.bandCentresHz.first, Double(caps.centres.first ?? 0))
        _ = t.takeRawOpcodesForTest()

        let id = vm.rta.subscribe(DSPi_Console.RtaRequest(tap: RTA_TAP_OUTPUT, mask: 0b1, drawsBins: true))
        XCTAssertTrue(spinUntil(10) { (vm.rta.snapshot.frames[0]?.nBands ?? 0) > 0 }, "a band frame of channel 0")
        XCTAssertTrue(spinUntil(10) { vm.rta.telemetry.status.isRunning }, "the device's analyser runs")
        XCTAssertTrue(spinUntil(10) { (vm.rta.snapshot.bins?.bins.count ?? 0) > 0 }, "the bins of one channel")
        XCTAssertEqual(vm.rta.snapshot.tap, RTA_TAP_OUTPUT)
        XCTAssertEqual(vm.rta.ownership, .driving)
        XCTAssertNotNil(vm.rta.renderState.frame(channel: 0, tap: RTA_TAP_OUTPUT), "the render thread has it too")
        XCTAssertFalse(vm.rta.configRejected)
        let levels = vm.rta.snapshot.frames[0]!.avg.prefix(Int(vm.rta.snapshot.frames[0]!.nBands))
        XCTAssertTrue(levels.contains { $0 > 0 }, "levels above the floor")

        // Two channels: frames of both, no bins.
        vm.rta.update(id, to: DSPi_Console.RtaRequest(tap: RTA_TAP_OUTPUT, mask: 0b110, drawsBins: true))
        // The device analyses those of them that are live; the view shows
        // what the core holds for them, and nothing of channel 0.
        XCTAssertTrue(spinUntil(10) {
            let shown = Set(vm.rta.snapshot.frames.keys)
            let held = Set(t.coreQueue.sync { core.rtaGetSnapshot() }.frames.prefix(Int(t.coreQueue.sync {
                core.rtaGetSnapshot() }.count)).map(\.channel))
            return !shown.isEmpty && shown.isSubset(of: [1, 2]) && shown == held
        }, "the channels asked for: \(vm.rta.snapshot.frames.keys.sorted())")
        XCTAssertTrue(spinUntil(5) { vm.rta.snapshot.bins == nil }, "bins only for one channel")

        // Take control while driving changes nothing.
        vm.rta.takeControl()
        settle()
        XCTAssertEqual(vm.rta.ownership, .driving)

        vm.rta.release(id)
        XCTAssertTrue(spinUntil(5) { vm.rta.snapshot.frames.isEmpty }, "nothing left to draw")
        XCTAssertTrue(spinUntil(5) { t.coreQueue.sync { core.rtaGetSnapshot().count } == 0 }, "the core released it")
        XCTAssertEqual(domainRaw(t), [])
    }

    /// The core's following state (CG5) as the engine shows it: the device's
    /// tap and channels, and who set them.
    func testSW14TheFollowingStateIsTheCores() throws {
        let followed = try XCTUnwrap(RtaEngine.following(fromCore: dspi_core.RtaOwnership(
            following: true, origin: 7, tap: RTA_TAP_INPUT, mask: 0b11, fftOrder: 9)))
        XCTAssertEqual(followed.origin, 7)
        XCTAssertEqual(followed.config.tap, RTA_TAP_INPUT)
        XCTAssertEqual(followed.config.channelMask, 0b11)
        XCTAssertEqual(followed.config.fftOrder, 9)
        XCTAssertNil(RtaEngine.following(fromCore: dspi_core.RtaOwnership(
            following: false, origin: 0, tap: RTA_TAP_OUTPUT, mask: 1, fftOrder: 10)), "driving")
    }

    /// The core's dBFS go back to the wire's level bytes the renderers read
    /// (its 0.5 dB steps exactly), its bins as the averaged and smoothed
    /// levels, its status as the status line's.
    func testSW14TheCoresSnapshotConverts() {
        var floor = Array(repeating: DspiConstants.RTA_FLOOR_DB, count: RTA_MAX_BANDS)
        floor[0] = 0
        floor[1] = -30
        let frame = RtaChannelFrame(channel: 3, nBands: 34, silent: false, avgDb: floor, peakDb: floor)
        let c = dspi_core.RtaSnapshot(
            version: 9, telemetryVersion: 2, tap: RTA_TAP_INPUT, count: 1, frames: [frame], populated: 0,
            binsCount: 2, binsChannel: 3, binsSampleRate: 48000, binsDb: [-20.25, -40, -121.5],
            running: true, liveCount: 1, liveMask: 0b1000, framesPerS: 47, busyUsPerS: 1000, lastFrameUs: 20,
            bassBusyUsPerS: 5, sampleRate: 48000, firstBand: 0, fftOrder: 10, configRejected: false,
            refreshIntervalS: 0.02)
        let s = RtaEngine.snapshot(fromCore: c)
        XCTAssertEqual(s.tap, RTA_TAP_INPUT)
        let f = s.frames[3]
        XCTAssertEqual(f?.nBands, 34)
        XCTAssertEqual(f?.avg[0], RTA_LEVEL_ZERO_DBFS, "0 dBFS")
        XCTAssertEqual(f?.avg[1], RTA_LEVEL_ZERO_DBFS - 60, "-30 dBFS in 0.5 dB steps")
        XCTAssertEqual(f?.avg[2], 0, "the floor")
        XCTAssertEqual(f?.avg.count, RTA_MAX_BANDS)
        XCTAssertTrue(f?.hasData ?? false)
        XCTAssertEqual(s.bins?.bins.count, 2)
        XCTAssertEqual(s.bins?.smoothedLevelsDB, [-20.25, -40])
        XCTAssertEqual(s.status.liveMask, 0b1000)
        XCTAssertTrue(s.status.isRunning)
        XCTAssertEqual(s.status.sampleRateHz, 48000)
        XCTAssertEqual(RtaEngine.snapshot(fromCore: dspi_core.RtaSnapshot(
            version: 0, telemetryVersion: 0, tap: 0, count: 0, frames: [], populated: 0, binsCount: 0,
            binsChannel: 0, binsSampleRate: 0, binsDb: [], running: false, liveCount: 0, liveMask: 0,
            framesPerS: 0, busyUsPerS: 0, lastFrameUs: 0, bassBusyUsPerS: 0, sampleRate: 0, firstBand: 0xFF,
            fftOrder: 0, configRejected: false, refreshIntervalS: 0)).frames.count, 0)
    }

    // MARK: - The Stats window

    /// Every read of the Stats window is the core's: the counters, the ring
    /// health, the buffer tick, the inputs' and outputs' status and the
    /// device's identity.
    func testSW14StatsThroughTheCore() {
        let (core, t, vm) = demoVM()
        let stats = StatsViewModel(transport: t)
        XCTAssertTrue(spinUntil(5) { stats.isConnected })
        _ = t.takeRawOpcodesForTest()
        stats.fetchDeviceInfo()
        XCTAssertTrue(spinUntil(5) { stats.platformName == vm.platformName && stats.firmwareVersion != "-" },
                      "identity from the core's state: \(stats.platformName) \(stats.firmwareVersion)")
        if let serial = t.selectedDevice?.serial, !serial.isEmpty {
            XCTAssertEqual(stats.serialNumber, serial)
        }
        stats.fetchStats()
        let rate = t.coreQueue.sync { try? core.fetchStat(which: DspiConstants.STAT_SAMPLE_RATE) }
        let clock = t.coreQueue.sync { try? core.fetchStat(which: DspiConstants.STAT_CLOCK_HZ) }
        XCTAssertTrue(spinUntil(5) { stats.sampleRateHz == rate && stats.systemClockHz == clock },
                      "the counters: \(stats.sampleRateHz) \(stats.systemClockHz)")
        let health = t.coreQueue.sync { try? core.fetchRingHealth() }
        XCTAssertTrue(spinUntil(5) { (stats.ringHealth == nil) == (health == nil) }, "ring health as the core's")
        if let h = health {
            XCTAssertEqual(stats.ringHealth?.targetFrames, h.targetFrames)
        }
        let lg = t.coreQueue.sync { try? core.fetchLgStatus() }
        XCTAssertTrue(spinUntil(5) { stats.lgSoundSyncSupported == (lg != nil) })

        stats.fetchBufferStats()
        let tick = t.coreQueue.sync { try? core.fetchStatsTick() }
        XCTAssertNotNil(tick)
        XCTAssertTrue(spinUntil(5) { stats.bufferStats.numSpdif == tick?.buffers.numSpdif })
        stats.resetBufferWatermarks()
        settle()
        XCTAssertEqual(domainRaw(t), [])
    }

    /// The core's stats tick as the window's packet: its ring-scaled fill to
    /// frame resolution.
    func testSW14TheStatsTickConverts() {
        let slot = dspi_core.SpdifBufferStats(free: 1, prepared: 2, playing: 3, fillPct: 50, minPct: 40, maxPct: 60)
        let fill = OutputFill(fillPct: 37.25, minPct: 30, maxPct: 45, band: .good)
        let tick = StatsTick(buffers: BufferStats(numSpdif: 1, pdmActive: true, streaming: true, sequence: 7,
                                                  spdif: [slot, slot, slot, slot],
                                                  pdm: dspi_core.PdmBufferStats(dmaFillPct: 1, dmaMinPct: 2, dmaMaxPct: 3,
                                                                      ringFillPct: 4, ringMinPct: 5,
                                                                      ringMaxPct: 6)),
                             fills: [fill])
        let p = BufferStatsPacket(core: tick)
        XCTAssertEqual(p.numSpdif, 1)
        XCTAssertTrue(p.pdmActive && p.audioStreaming)
        XCTAssertEqual(p.sequence, 7)
        XCTAssertEqual(p.spdif[0].consumerPrepared, 2)
        XCTAssertEqual(p.spdif[0].consumerFillPct, 37)
        XCTAssertEqual(p.spdif[0].preciseFillPct, 37.25, accuracy: 0.001)
        XCTAssertEqual(p.spdif[0].consumerMinFillPct, 30)
        XCTAssertEqual(p.spdif[1].consumerFillPct, 0, "only the live slots")
        XCTAssertEqual(p.pdm.ringMaxFillPct, 6)
    }

    // MARK: - The Interrupt Monitor

    /// Through a core the log shows the core's lines (`monitorRead`), which
    /// name what changed, each once.
    func testSW14TheMonitorLogIsTheCores() {
        let (core, t, _) = demoVM()
        let monitor = InterruptMonitor(transport: t)
        monitor.start()
        _ = t.takeRawOpcodesForTest()
        t.coreQueue.sync { try? core.demoRenameElsewhere(target: 1, name: "SW14 monitor") }
        XCTAssertTrue(spinUntil(5) { monitor.events.contains { $0.text != nil } }, "a line of the core's log")
        let lines = t.coreQueue.sync { core.monitorRead(afterId: 0).entries.map(\.text) }
        for event in monitor.events {
            let text = try? XCTUnwrap(event.text)
            XCTAssertTrue(lines.contains(text ?? ""), "the core's own line: \(text ?? "")")
            XCTAssertTrue(event.rawBytes.isEmpty)
            XCTAssertTrue(event.formattedLine.hasSuffix(text ?? "?"))
        }
        let shown = monitor.events.count
        // A restart on the same core shows no line twice.
        monitor.start()
        t.coreQueue.sync { try? core.demoRenameElsewhere(target: 1, name: "SW14 again") }
        XCTAssertTrue(spinUntil(5) { monitor.events.count > shown })
        XCTAssertEqual(Set(monitor.events.map(\.text)).count, monitor.events.count, "each line once")
        monitor.stop()
        XCTAssertEqual(domainRaw(t), [])
    }

    // MARK: - Review sw13's nits

    /// N2: the core's missing-channels line names eight and counts the rest;
    /// the count is not a channel.
    func testSW14TheCoresMissingChannelsLineParses() {
        let named = PresetDocumentApply.missingChannels(in: "S/PDIF 3 L, S/PDIF 3 R, and 3 more")
        XCTAssertEqual(named.names, ["S/PDIF 3 L", "S/PDIF 3 R"])
        XCTAssertEqual(named.more, 3)
        let all = PresetDocumentApply.missingChannels(in: "PDM, Sub")
        XCTAssertEqual(all.names, ["PDM", "Sub"])
        XCTAssertEqual(all.more, 0)
        let (report, _) = PresetDocumentApply.report(fromCoreReply: """
            {"ok":true,"clean":false,"lines":["Applied 2 channels, 3 EQ bands, 0 crossover bands, 4 crosspoints.",            "Not present on this device: a, b, c, d, e, f, g, h, and 1 more"]}
            """)
        XCTAssertEqual(report.missingChannels.count, 8)
        XCTAssertEqual(report.missingChannelsMore, 1)
        XCTAssertFalse(report.missingChannels.contains { $0.contains("more") })
    }

    /// N3: an export the core refused says so, not "Failed to write file".
    func testSW14AnExportTheCoreRefusedIsNotAWriteFailure() {
        let text = FileMenuActions.configurationExportFailure(DspiError.Transfer(msg: "no settings have been read"))
        XCTAssertEqual(text, "Could not export the configuration: no settings have been read")
        XCTAssertFalse(FileMenuActions.configurationExportFailure(DspiError.Unsupported).contains("write file"))
    }

    // MARK: - No raw requests in this domain

    /// Every function of the domain with a core attached: none of its
    /// requests goes out raw (DEBUG record of the app's own raw requests).
    func testSW14EveryDomainCallGoesThroughTheCore() {
        let (_, t, vm) = rtaVM()
        XCTAssertTrue(spinUntil(10) { vm.siggenSupported })
        let stats = StatsViewModel(transport: t)
        XCTAssertTrue(spinUntil(5) { stats.isConnected })
        let monitor = InterruptMonitor(transport: t)
        _ = t.takeRawOpcodesForTest()

        offMain {
            vm.fetchStatus()
            vm.fetchSampleRate()
            vm.clearClips()
            vm.fetchSiggen()
            vm.fetchSiggenStatus()
            _ = vm.siggenControl(SIGGEN_CTL_STOP)   // a stop only; nothing is started here
            vm.rta.fetchCaps()
            vm.rta.tick()
        }
        vm.siggenSetConfig(vm.siggenDraft)
        let id = vm.rta.subscribe(DSPi_Console.RtaRequest(tap: RTA_TAP_INPUT, mask: 0b1, drawsBins: true))
        vm.rta.setOptions(RtaOptions(fftOrder: 9, avgMs: 500, peakDecayDBs: 6))
        vm.rta.takeControl()
        vm.rta.deviceAnnounced(RtaConfig(tap: RTA_TAP_INPUT, channelMask: 0b10), origin: 7)
        XCTAssertTrue(spinUntil(10) { vm.rta.snapshot.frames[0] != nil })
        vm.rta.release(id)
        stats.fetchDeviceInfo()
        stats.fetchStats()
        stats.fetchBufferStats()
        stats.resetBufferWatermarks()
        monitor.start()
        settle(1)
        monitor.stop()
        flush(t)
        XCTAssertEqual(domainRaw(t), [])
        XCTAssertEqual(vm.rta.ownership, .driving, "an event handed to the engine is the core's to judge")
    }
}
