//
//  CoreDiagnostics.swift
//  DSPi Console
//
//  WP SW14.  The core's records of the analysis and diagnostics domain (the
//  signal generator, the Stats window's reads, the meters) turned into the
//  Swift models the views already draw, for the paths with a core attached
//  (docs/SWIFT-SEAMS.md section 3, SW14).  The analyser's own conversion is
//  in SpectrumAnalyser.swift, beside the engine it feeds.
//

import Foundation
import dspi_core
import dspi_constants

// The signal generator (Commands.swift "Signal Generator").
extension SiggenConfig {
    init(core c: dspi_core.SiggenConfig) {
        func p(_ i: Int) -> Float { i < c.p.count ? c.p[i] : 0 }
        self.init(signalType: c.signalType, channelMask: c.channelMask, invertMask: c.invertMask, flags: c.flags,
                  levelDB: c.levelDb, durationMS: c.durationMs, repeatCount: c.`repeat`, gapMS: c.gapMs,
                  p1: p(0), p2: p(1), p3: p(2), p4: p(3))
    }

    /// The core's record of this config (`p` has the four parameters).
    var core: dspi_core.SiggenConfig {
        dspi_core.SiggenConfig(signalType: signalType, channelMask: channelMask, invertMask: invertMask,
                               flags: flags, levelDb: levelDB, durationMs: durationMS, repeat: repeatCount,
                               gapMs: gapMS, p: [p1, p2, p3, p4])
    }
}

extension SiggenStatus {
    init(core s: dspi_core.SiggenStatus) {
        self.init(state: s.state, signalType: s.signalType, activeChannel: s.activeChannel, elapsedMS: s.elapsedMs,
                  cyclesDone: s.cyclesDone, stopReason: s.stopReason, currentFreq: s.currentFreq)
    }
}

extension SiggenCapsHeader {
    init(core c: dspi_core.SiggenCaps) {
        self.init(version: SIGGEN_CFG_VERSION, typeCount: c.typeCount, outputChannels: c.outputChannels,
                  multitoneMax: c.multitoneMax, validChannelMask: c.validMask)
    }
}

extension SiggenTypeDesc {
    init(core t: dspi_core.SiggenTypeInfo) {
        var params = t.params.prefix(4).map {
            SiggenParamDesc(semantic: $0.semantic, min: $0.min, max: $0.max, def: $0.def)
        }
        while params.count < 4 { params.append(SiggenParamDesc()) }
        self.init(id: t.id, name: t.name, timingModel: t.timing, params: params)
    }
}


// The Stats window (StatsView.swift).
extension RingHealth {
    init(core h: dspi_core.RingHealth) {
        self.init(targetFrames: h.targetFrames, targetUs: h.targetUs, batchFrames: h.batchFrames,
                  underrunEpisodes: h.underrunEpisodes, refusedBlocks: h.refusedBlocks,
                  lowestPrewriteFill: h.lowestFill, worstInputGapUs: h.worstInputGapUs,
                  worstUsbPacketGapUs: h.worstUsbGapUs, overBudgetSteps: h.overBudgetSteps)
    }
}

extension SpdifRxStatus {
    init(core r: dspi_core.SpdifRxStatus) {
        self.init(state: r.state, inputSource: r.inputSource, lockCount: r.lockCount, lossCount: r.lossCount,
                  sampleRate: r.sampleRate, parityErrors: r.parityErrors, fifoFillPct: r.fifoFillPct,
                  libState: r.libState,
                  callbackCounts: (r.stableCallbacks & 0x0F) << 4 | (r.lostCallbacks & 0x0F))
    }
}

extension BufferStatsPacket {
    /// The core's stats tick.  Its outputs' fill is on the ring's own scale
    /// already, to frame resolution (CG12), so it replaces the percentages
    /// and the precise fill of each live S/PDIF slot.
    init(core t: dspi_core.StatsTick) {
        let b = t.buffers
        self.init()
        numSpdif = b.numSpdif
        flags = (b.pdmActive ? 0x01 : 0) | (b.streaming ? 0x02 : 0)
        sequence = b.sequence
        let fills = t.fills ?? []
        for i in 0..<min(Int(b.numSpdif), 4, b.spdif.count) {
            let s = b.spdif[i]
            var stats = SpdifBufferStats(consumerFree: s.free, consumerPrepared: s.prepared,
                                         consumerPlaying: s.playing, consumerFillPct: s.fillPct,
                                         consumerMinFillPct: s.minPct, consumerMaxFillPct: s.maxPct)
            if i < fills.count {
                let f = fills[i]
                let pct = max(0, f.fillPct)
                stats.consumerFillPct = UInt8(clamping: Int(pct.rounded()))
                stats.consumerMinFillPct = f.minPct
                stats.consumerMaxFillPct = f.maxPct
                stats.fillCentiPct = UInt16(clamping: Int((pct * 100).rounded()))
            }
            spdif[i] = stats
        }
        pdm = PdmBufferStats(dmaFillPct: b.pdm.dmaFillPct, dmaMinFillPct: b.pdm.dmaMinPct,
                             dmaMaxFillPct: b.pdm.dmaMaxPct, ringFillPct: b.pdm.ringFillPct,
                             ringMinFillPct: b.pdm.ringMinPct, ringMaxFillPct: b.pdm.ringMaxPct)
    }
}

// -- WP SW14 -- review sw13 N3: why an export made no document.
extension PresetDocumentExport {
    /// Through a core, its refusal in its words; otherwise the error's own.
    static func failureReason(_ error: Error) -> String {
        switch error as? DspiError {
        case .Transfer(let msg)?: return msg
        case .Unsupported?: return "this device's settings cannot be written to a configuration file"
        case .Disconnected?: return "the device is not connected"
        default: return error.localizedDescription
        }
    }
}
