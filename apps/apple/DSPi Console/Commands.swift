import Foundation
import Combine
import SwiftUI
import dspi_core
import dspi_constants

extension DSPViewModel {

    // --- USB Commands ---

    /// Full parameter refresh.  Pass `afterConnect: true` on the connect path,
    /// where the device may not be ready to answer yet and the reads must be
    /// retried instead of treated as a dead link (see `waitForDeviceReady`).
    /// Blocking, so `afterConnect` callers must be off the main thread.
    func fetchAll(afterConnect: Bool = false) {
        // Scope this refresh to the device that is open right now. If a
        // device switch lands mid-fetch, later reads would return the NEW
        // device's data and publish a mix of both devices' state - bail out
        // instead; the switch itself triggers a fresh fetchAll that does the
        // full refresh.
        let generation = transport.generation

        // Fetch platform/version first so capability gates (notch filter,
        // per-band bypass, etc.) are populated before the UI reads them.
        // It doubles as the readiness probe on the connect path: it is the
        // cheapest interface-recipient read, i.e. the same path the bulk
        // transfer below uses.
        if afterConnect {
            guard waitForDeviceReady(generation: generation) else {
                // Never answered - the link really is dead.
                self.transport.markDisconnected()
                return
            }
        } else {
            _ = fetchPlatform()
        }

        let gotParams = afterConnect
            ? fetchAllParamsRetrying(generation: generation)
            : fetchAllParams()
        guard gotParams else { return }
        guard transport.generation == generation else { return }

        // Tier 1: only what the main window renders - source, rate, volume
        // and the preset picker.  The bulk read above already carried the
        // filters and channel layout.
        // -- WP SW8 -- Through the core, its fetchAll read the volumes and the
        // preset directory and names with the image.
        let throughCore = transport.coreStateSource != nil
        fetchInputSource()
        fetchSampleRate()
        if !throughCore { fetchUserVolume() }
        guard transport.generation == generation else { return }

        // Fetch preset state
        if !throughCore {
            let occupied = fetchPresetDirectory()
            for slot in 0..<10 {
                if (occupied & UInt16(1 << slot)) != 0 {
                    fetchPresetName(slot: slot)
                } else {
                    DispatchQueue.main.async {
                        self.presetNames[slot] = ""
                    }
                }
            }
            fetchPresetActive()
        }
        guard transport.generation == generation else { return }

        // The live UI fades in here, not on raw connection and not after the
        // whole fetch: everything the main window shows has published by now,
        // so the content arrives fully formed, and the far larger tier below
        // (control surfaces alone is ~140 transfers) no longer holds the
        // fade hostage.
        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.3)) { self.isDeviceReady = true }
        }

        // Tier 2: state for Settings pages and tool windows, which are almost
        // never open at the moment of connection.  Their published values
        // simply land a beat after the console appears; the gates they feed
        // (controlSurfacesSupported, siggenSupported) materialise with them.
        fetchCore1Mode()
        fetchLgSoundSyncEnabled()
        fetchDacHwMuteConfig()
        fetchOutputLatency()
        fetchControlInterfaces()
        fetchControlSurfaces()
        guard transport.generation == generation else { return }

        fetchSiggen()
        // The analyser's caps read is its whole feature gate: it is transient
        // and absent from the bulk blob, so there is no wire-format version to
        // key off.  Firmware without it STALLs the read and every analyser view
        // shows its unsupported notice.
        rta.fetchCaps()
        fetchAdatConfig()
        fetchAdatInputConfig()
        // Extension channel: one EXT_LIST read per connect (a refresh reuses it).
        probeExtensions(force: afterConnect)
        guard transport.generation == generation else { return }

        // All fetches above have enqueued their main-thread state updates, so
        // this runs after the published values reflect the connected device.
        // Re-seeds the Settings global draft (no-op if the user has staged
        // edits) - matters after a device switch, where the draft was reset
        // from the previous device's values.  Deliberately after tier 2: the
        // draft snapshots values that tier fetches.
        DispatchQueue.main.async {
            SettingsSaveCoordinator.shared.refreshGlobalDraftIfClean()
            self.fullReadGeneration = generation   // -- WP SWF1 --
        }
    }

    /// Polls a cheap vendor read until the device answers.
    ///
    /// A freshly enumerated device is not immediately usable: macOS publishes
    /// the IOKit service before the configuration is set, so interface-recipient
    /// control requests can fail for a while, and a just-flashed unit is still
    /// initialising its stored parameters on top of that.  Every read in the
    /// connect path is one-shot - `fetchAllParams` clears `isConnected` when it
    /// fails and nothing retries afterwards - so wait for the device to prove
    /// it is answering first.
    ///
    /// Returns false only if the device never responded, or if the connection
    /// changed underneath us (in which case the caller must bail out silently).
    private func waitForDeviceReady(generation: UInt64) -> Bool {
        let backoff: [TimeInterval] = [0, 0.1, 0.2, 0.4, 0.8, 1.0]
        for delay in backoff {
            if delay > 0 { Thread.sleep(forTimeInterval: delay) }
            guard transport.generation == generation else { return false }
            if fetchPlatform() != nil { return true }
        }
        return false
    }

    /// `fetchAllParams` for the connect path: retries the bulk transfer a few
    /// times before letting it mark the device disconnected.  The device can
    /// answer the small readiness probe and still stall on the ~6 KB bulk read
    /// while it finishes booting.
    private func fetchAllParamsRetrying(generation: UInt64) -> Bool {
        let backoff: [TimeInterval] = [0.15, 0.3, 0.6]
        for delay in backoff {
            // A short/wrong-version payload also returns false here, but the
            // device is alive and the firmware simply incompatible; retrying
            // costs a few reads and the final call records the version.
            if fetchAllParams(markDisconnectedOnFailure: false) { return true }
            guard transport.generation == generation else { return false }
            Thread.sleep(forTimeInterval: delay)
            guard transport.generation == generation else { return false }
        }
        return fetchAllParams()
    }

    @discardableResult
    func fetchPlatform() -> String? {
        // Decoding, including the short replies from older firmware and the
        // early 1.1.6 betas, lives in FirmwareVersion.fromPlatformReply.  See
        // the firmware's firmware_versioning_spec.md for the wire layout.
        guard let data = transport.getControlRequest(request: REQ_GET_PLATFORM, value: 0, index: 2, length: 7),
              let reply = FirmwareVersion.fromPlatformReply([UInt8](data)) else { return nil }
        let platform = reply.platform
        let (major, minor, patch, beta) = (reply.version.major, reply.version.minor,
                                           reply.version.patch, reply.version.beta)
        let name: String
        switch platform {
        case 1:  name = "RP2350"
        case 2:  name = "STM32H723"
        default: name = "RP2040"
        }
        DispatchQueue.main.async {
            self.platformName = name
            self.firmwareVersion = (major: major, minor: minor, patch: patch, beta: beta)
        }
        return name
    }

    func fetchStatus() {
        let numChannels = self.numChannels
        // V16 combined status (wValue=9): peaks[numCh] u16 + cpu0 + cpu1 +
        // clip_flags (u32) + active_input_channels (u8).
        let responseSize = numChannels * 2 + 2 + 4 + 1

        guard let data = transport.getControlRequest(
            request: REQ_GET_STATUS, value: 9, index: 0,
            length: UInt16(responseSize)
        ), data.count >= responseSize else { return }

        var peaks = [Float](repeating: 0, count: numChannels)
        for i in 0..<numChannels {
            let raw = data.withUnsafeBytes { $0.load(fromByteOffset: i * 2, as: UInt16.self) }
            peaks[i] = Float(raw) / 32767.0
        }
        let off = numChannels * 2
        let cpu0 = Int(data[off])
        let cpu1 = Int(data[off + 1])
        let clipFlags = data.withUnsafeBytes {
            $0.load(fromByteOffset: off + 2, as: UInt32.self)
        }
        // Live active input count (2/4/6/8); follows the host's USB audio format.
        let activeInputs = max(BASE_MATRIX_INPUTS, Int(data[off + 6]))

        DispatchQueue.main.async {
            if self.activeInputChannels != activeInputs {
                self.activeInputChannels = activeInputs
            }
            var s = self.meters.status
            var fullPeaks = Array(repeating: Float(0), count: WIRE_MAX_CHANNELS)
            for i in 0..<min(numChannels, WIRE_MAX_CHANNELS) { fullPeaks[i] = peaks[i] }
            s.peaks = fullPeaks
            s.cpu0 = cpu0
            s.cpu1 = cpu1
            s.clipFlags = clipFlags

            // Only update timestamp when genuinely new clip bits appear
            let newClips = clipFlags & ~s.clipLatched
            s.clipLatched |= clipFlags
            if newClips != 0 {
                s.clipTimestamp = Date()
            }

            // Auto-clear after 10 seconds of no new clips
            if let ts = s.clipTimestamp,
               Date().timeIntervalSince(ts) > 10.0,
               s.clipLatched != 0 {
                s.clipLatched = 0
                s.clipTimestamp = nil
                DispatchQueue.global(qos: .utility).async { self.clearClips() }
            }
            // Silence and a stopped stream repeat the same reading; only a
            // change is worth an update.
            if s != self.meters.status { self.meters.status = s }
        }

        // Poll sample rate at a lower cadence than meter traffic to keep
        // hardware UI constraints current (e.g., MCK multiplier gating).
        struct SampleRatePollState { static var decimator = 0 }
        SampleRatePollState.decimator = (SampleRatePollState.decimator + 1) & 0x0F
        if SampleRatePollState.decimator == 0 {
            fetchSampleRate()
        }
    }

    func fetchSampleRate() {
        guard let data = transport.getControlRequest(request: REQ_GET_STATUS, value: 15, index: 0, length: 4),
              data.count >= 4 else { return }
        let rate = data.withUnsafeBytes { $0.load(as: UInt32.self) }
        DispatchQueue.main.async {
            // @Published fires on assignment, not on change; the rate repeats
            // on every poll, so only publish when it actually moves.
            if self.telemetry.sampleRateHz != rate { self.telemetry.sampleRateHz = rate }
            // Keep UI consistent with firmware/runtime policy:
            // 256x MCK is not available at 96 kHz.
            if rate >= 96000, self.mckMultiplier == 256 {
                self.mckMultiplier = 128
            }
        }
    }

    func clearClips() {
        _ = transport.getControlRequest(request: REQ_CLEAR_CLIPS, value: 0, index: 0, length: 2)
    }

    // MARK: - USB-only sends (no @Published update, for slider drag)

    // -- WP SW9 -- Through a core, these sends are its typed setters behind
    // the write lane (SWIFT-SEAMS 5.2): while a value for the same setting is
    // still queued, a newer one replaces it, so a drag sends at most one
    // write per transfer time; a write issued for an earlier connection, or
    // queued behind a failed one, is dropped, never replayed.  The release
    // commits the last value with the ordinary setter below.  Without a core
    // (USB without -CoreTransport, MIDI, the Link client) they stay raw SETs.

    /// The core this view model's device is driven through, if any.
    var typedCore: CoreStateSource? { transport.coreStateSource }

    func sendPreampToDevice(_ db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        if let core = typedCore {
            core.write(lane: "pre.all") { [val] in try $0.setPreamp(db: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PREAMP, value: 0, index: 0, data: data)
    }

    func sendPreampChannelToDevice(channel: Int, db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        // A linked pair gets one per-channel write each; never the legacy
        // all-channel REQ_SET_PREAMP, which would clobber the trims of the
        // inputs outside this pair.
        if let core = typedCore {
            for input in [channel] + (linkedPartner(of: channel).map { [$0] } ?? []) {
                core.write(lane: "pre[\(input)]") { [val] in try $0.setInputPreamp(input: Self.wire(input), db: val) }
            }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PREAMP_CH, value: UInt16(channel), index: 0, data: data)
        if let partner = linkedPartner(of: channel) {
            transport.sendControlRequest(request: REQ_SET_PREAMP_CH, value: UInt16(partner), index: 0, data: data)
        }
    }

    func sendMasterVolumeToDevice(_ db: Float) {
        var val = keptMasterVolume(db)
        if let core = typedCore {
            core.write(lane: "vol.master") { [val] in try $0.setMasterVolume(db: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0, data: data)
    }

    /// -- WP SW9 -- An index as the core's byte (it refuses one out of range).
    static func wire(_ index: Int) -> UInt8 { UInt8(clamping: index) }

    /// The master volume as it will be kept: through a core, the core's own
    /// rule (0.5 dB steps, -128 mutes), so the slider shows the value the
    /// device gets; else this app's finer steps near 0 dB.
    func keptMasterVolume(_ db: Float) -> Float {
        guard typedCore != nil else { return Self.roundMasterVolume(db) }
        if db <= -128 { return -128 }
        let val = min(max((db * 2).rounded() / 2, -127), 0)
        return val == -0.0 ? 0.0 : val
    }

    private static func roundMasterVolume(_ db: Float) -> Float {
        if db <= -128 { return -128 }
        if db >= 0 { return 0 }
        let rounded: Float
        if db > -10 {
            rounded = (db * 10).rounded() / 10
        } else if db > -40 {
            rounded = (db * 2).rounded() / 2
        } else {
            rounded = db.rounded()
        }
        return rounded == -0.0 ? 0.0 : rounded
    }

    /// Drag-time output gain.  The device follows every 0.1 dB step, but only
    /// the response graph sees it, through `outputGainPreview`: publishing
    /// `outputGainDB` on every mouse movement re-evaluated each view observing
    /// the model and starved the main thread.  The release commits the value
    /// once through `setOutputGain`.
    func sendOutputGainToDevice(output: Int, db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        guard outputGainPreview.gains[output] != val else { return }
        outputGainPreview.gains[output] = val
        if let core = typedCore {
            core.write(lane: "out.gain[\(output)]") { [val] in try $0.setOutputGain(output: Self.wire(output), db: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_GAIN, value: UInt16(output), index: 2, data: data)
    }

    func sendOutputDelayToDevice(output: Int, ms: Float) {
        var val = Self.quantizeDelayMS(min(ms, maxOutputDelayMS))   // -- WP SWF5 -- the slider rounds its drag
        if let core = typedCore {
            core.write(lane: "out.delay[\(output)]") { [val] in try $0.setOutputDelay(output: Self.wire(output), ms: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_DELAY, value: UInt16(output), index: 2, data: data)
    }

    /// -- WP SW9 -- A band's gain on the grid it is kept on: through a core,
    /// the core's 0.1 dB (`quantize_gain`; a Linkwitz Transform's fp in Hz
    /// is sent as given), so the value shown is the one the device gets;
    /// else 0.001 dB.
    func bandGain(_ p: FilterParams) -> Float {
        let step: Float = typedCore != nil && p.type != .linkwitzTransform ? 10 : 1000
        let g = (p.gain * step).rounded() / step
        return g == -0.0 ? 0.0 : g
    }

    /// -- WP SW9 -- A band as the core's record.  qp is the Linkwitz
    /// Transform's; the core ignores it for every other type.
    static func coreBand(_ p: FilterParams) -> dspi_core.FilterParams {
        dspi_core.FilterParams(filterType: UInt8(clamping: p.type.rawValue), bypass: p.bypass,
                               freq: p.freq, q: p.q, gain: p.gain, qp: p.qp)
    }

    func setFilter(ch: Int, band: Int, p: FilterParams) {
        var p = p
        // Gain is quantized to 0.001 dB - the precision the band row shows.  A
        // grid (rather than the raw float) keeps the value the device echoes
        // back identical to the one we sent, so preset dirty-tracking compares
        // cleanly.  For a Linkwitz Transform this field is fp in Hz, so the
        // same grid applies there.  -- WP SW9 -- Through a core, its grid.
        p.gain = bandGain(p)
        channelData[ch]?[band] = p
        if let core = typedCore {
            let wire = Self.coreBand(p)
            core.commit(lane: "eq[\(ch)][\(band)]") {   // -- fixup sw9 F2 -- behind the drag's lane
                try $0.setFilter(ch: Self.wire(ch), band: Self.wire(band), params: wire)
            }
        } else {
            transport.sendControlRequest(request: REQ_SET_EQ_PARAM, value: 0, index: 0, data: eqParamPacket(ch: ch, band: band, p: p))
        }
        recomputeMagnitudes(for: ch)
    }

    /// Drag-time band write: the device follows, the model does not publish.
    /// The graph editor draws the live curve itself and commits once through
    /// `setFilter` on release, the same split as `sendOutputGainToDevice`.
    func sendFilterToDevice(ch: Int, band: Int, p: FilterParams) {
        var p = p
        p.gain = bandGain(p)
        if let core = typedCore {   // -- WP SW9 --
            let wire = Self.coreBand(p)
            core.write(lane: "eq[\(ch)][\(band)]") {
                try $0.setFilter(ch: Self.wire(ch), band: Self.wire(band), params: wire)
            }
            return
        }
        transport.sendControlRequest(request: REQ_SET_EQ_PARAM, value: 0, index: 0, data: eqParamPacket(ch: ch, band: band, p: p))
    }

    private func eqParamPacket(ch: Int, band: Int, p: FilterParams) -> Data {
        // EqParamPacket layout: [ch, band, type, bypass, freq(4), q(4), gain(4)] = 16 bytes.
        // Per band_bypass_spec §5: write exactly 0 or 1 — never 0xFF.
        // For the Linkwitz Transform (type 11) append the 2-byte `qp` sidecar
        // (Qp x 512), making an 18-byte payload; the firmware then latches the
        // target Q.  The 16-byte form preserves the stored qp, so we always send
        // the long form for LT bands.  See peq_filters.md §4.1.
        let data = NSMutableData()
        var ch8 = UInt8(ch); data.append(&ch8, length: 1)
        var b8 = UInt8(band); data.append(&b8, length: 1)
        var t8 = UInt8(p.type.rawValue); data.append(&t8, length: 1)
        var bp = UInt8(p.bypass ? 1 : 0); data.append(&bp, length: 1)
        var f32 = p.freq; data.append(&f32, length: 4)
        var q32 = p.q; data.append(&q32, length: 4)
        var g32 = p.gain; data.append(&g32, length: 4)
        if p.type == .linkwitzTransform {
            var qp16 = p.qpEncoded; data.append(&qp16, length: 2)
        }
        return data as Data
    }
    
    /// Toggle a single band's bypass flag without touching freq/Q/gain.
    /// Cheaper than REQ_SET_EQ_PARAM when the user just clicks the bypass
    /// checkbox, and avoids racing with an in-flight parameter edit.
    func setBandBypass(ch: Int, band: Int, bypass: Bool) {
        // Update local cache immediately so the UI is responsive; the
        // notification echo from the firmware will re-confirm.
        if var bands = channelData[ch], band < bands.count {
            bands[band].bypass = bypass
            channelData[ch] = bands
            recomputeMagnitudes(for: ch)
        }

        sendBandBypass(ch: ch, wireBand: band, bypass: bypass)
    }

    /// -- WP SW9 -- REQ_SET_BAND_BYPASS for a PEQ band (0..9) or a crossover
    /// band (20..23).
    private func sendBandBypass(ch: Int, wireBand band: Int, bypass: Bool) {
        if let core = typedCore {
            core.write(lane: nil) { try $0.setBandBypass(ch: Self.wire(ch), band: Self.wire(band), bypass: bypass) }
            return
        }
        let wValue = UInt16((ch << 8) | band)
        let payload = Data([bypass ? 1 : 0])
        transport.sendControlRequest(request: REQ_SET_BAND_BYPASS, value: wValue, index: 0, data: payload)
    }

    // MARK: - Crossover Bands
    //
    // Crossover uses the existing EQ-band addressing (REQ_SET/GET_EQ_PARAM)
    // with band indices 20..23 reserved for the four crossover bands per
    // output channel.  See crossover_filters_spec.md §3.  The wValue layout
    // for SET is unused; the recipe is in the 16-byte payload.

    /// Set a single crossover band on an output channel.  `localBand` is 0..3;
    /// the wire band index is 20+localBand.  No-op for master channels (the
    /// firmware would reject them anyway) and for invalid band indices.
    func setCrossoverBand(ch: Int, localBand: Int, p: FilterParams) {
        guard ch >= chOut1,
              localBand >= 0,
              localBand < DSPViewModel.crossoverBandsPerChannel else { return }

        let wireBand = DSPViewModel.crossoverWireBand(localBand)
        var p = p
        // Crossover types ignore Q / gain in firmware; we keep the values for
        // round-trip but normalize so saved snapshots compare cleanly.
        p.gain = 0
        if p.q <= 0 { p.q = 0.707 }
        if var bands = xoverData[ch], localBand < bands.count {
            bands[localBand] = p
            xoverData[ch] = bands
            recomputeMagnitudes(for: ch)
        }

        if let core = typedCore {   // -- WP SW9 --
            let wire = Self.coreBand(p)
            core.write(lane: nil) {
                try $0.setCrossover(ch: Self.wire(ch), xband: Self.wire(localBand), params: wire)
            }
            return
        }

        // EqParamPacket: [ch, band, type, bypass, freq(4), q(4), gain(4)] = 16
        let data = NSMutableData()
        var ch8 = UInt8(ch); data.append(&ch8, length: 1)
        var b8 = UInt8(wireBand); data.append(&b8, length: 1)
        var t8 = UInt8(p.type.rawValue); data.append(&t8, length: 1)
        var bp = UInt8(p.bypass ? 1 : 0); data.append(&bp, length: 1)
        var f32 = p.freq; data.append(&f32, length: 4)
        var q32 = p.q;    data.append(&q32, length: 4)
        var g32 = p.gain; data.append(&g32, length: 4)

        transport.sendControlRequest(request: REQ_SET_EQ_PARAM, value: 0, index: 0, data: data as Data)
    }

    /// Toggle a single crossover band's bypass flag.  Uses REQ_SET_BAND_BYPASS
    /// (0xD8) — same opcode as PEQ.  No-op for master channels.
    func setCrossoverBandBypass(ch: Int, localBand: Int, bypass: Bool) {
        guard ch >= chOut1,
              localBand >= 0,
              localBand < DSPViewModel.crossoverBandsPerChannel else { return }
        let wireBand = DSPViewModel.crossoverWireBand(localBand)
        if var bands = xoverData[ch], localBand < bands.count {
            bands[localBand].bypass = bypass
            xoverData[ch] = bands
            recomputeMagnitudes(for: ch)
        }
        sendBandBypass(ch: ch, wireBand: wireBand, bypass: bypass)
    }

    /// Clear all crossover bands on an output channel by writing the default
    /// (FLAT, 1000 Hz, Q=0.707, gain=0, bypass=0) recipe to each.
    func clearCrossoverBands(ch: Int) {
        guard ch >= chOut1 else { return }
        for i in 0..<DSPViewModel.crossoverBandsPerChannel {
            let defaults = FilterParams(type: .flat, freq: 1000, q: 0.707, gain: 0)
            setCrossoverBand(ch: ch, localBand: i, p: defaults)
        }
    }

    /// Clear all PEQ bands on a single channel by writing the default
    /// (FLAT, 1000 Hz, Q=0.707, gain=0) recipe to each.  Used by the Clear All
    /// button on output channel PEQ tabs.
    func clearPEQBands(ch: Int) {
        let bandCount = channelData[ch]?.count ?? 10
        let defaults = FilterParams(type: .flat, freq: 1000, q: 0.707, gain: 0)
        for b in 0..<bandCount {
            setFilter(ch: ch, band: b, p: defaults)
        }
        recomputeMagnitudes(for: ch)
    }

    // -- WP SWF5 -- Delays keep the core's 0.0001 ms grid (`quantize_delay`)
    // instead of upstream's whole ms, so a 0.5 ms delay from a preset file,
    // another console or the device is written as 0.5 ms.  The views' sliders
    // and scroll steps round to whole ms themselves; the core clamps.
    static func quantizeDelayMS(_ ms: Float) -> Float {
        let val = (ms * 10000).rounded() / 10000
        return val == 0 ? 0 : val
    }

    func setDelay(ch: Int, ms: Float) {
        var val = Self.quantizeDelayMS(ms)
        self.channelDelays[ch] = val
        if let core = typedCore {   // -- WP SW9 --
            core.write(lane: nil) { [val] in try $0.setDelay(ch: Self.wire(ch), ms: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_DELAY, value: UInt16(ch), index: 0, data: data)
    }

    /// Legacy global preamp SET — sets both channels to the same value via 0x44.
    func setPreamp(_ db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        // The device sets every input, so every entry follows.  Replacing the
        // array with two entries would leave inputs 2+ out of range.
        self.preampDB = Array(repeating: val, count: preampDB.count)
        if let core = typedCore {   // -- WP SW9 --
            core.commit(lane: "pre.all") { [val] in try $0.setPreamp(db: val) }   // -- fixup sw9 F2 --
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PREAMP, value: 0, index: 0, data: data)
    }

    // MARK: - Per-Channel Preamp

    func setPreampChannel(channel: Int, db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        self.preampDB[channel] = val
        let partner = linkedPartner(of: channel)
        if let partner = partner { self.preampDB[partner] = val }
        // A linked pair gets one per-channel write each; never the legacy
        // all-channel REQ_SET_PREAMP, which would clobber the trims of the
        // inputs outside this pair.
        if let core = typedCore {   // -- WP SW9 --
            for input in [channel] + (partner.map { [$0] } ?? []) {
                core.commit(lane: "pre[\(input)]") { [val] in   // -- fixup sw9 F2 --
                    try $0.setInputPreamp(input: Self.wire(input), db: val)
                }
            }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PREAMP_CH, value: UInt16(channel), index: 0, data: data)
        if let partner = partner {
            transport.sendControlRequest(request: REQ_SET_PREAMP_CH, value: UInt16(partner), index: 0, data: data)
        }
    }

    // MARK: - Master Volume

    func setMasterVolume(_ db: Float) {
        var val = keptMasterVolume(db)
        self.masterVolumeDB = val
        if let core = typedCore {   // -- WP SW9 --
            core.commit(lane: "vol.master") { [val] in try $0.setMasterVolume(db: val) }   // -- fixup sw9 F2 --
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_MASTER_VOLUME, value: 0, index: 0, data: data)
    }

    // MARK: - User Volume (vendor channel for audio_state.volume)

    /// Send a user-volume value (firmware applies it to the same field
    /// the UAC1 host slider drives — works across USB / SPDIF / I2S).
    /// Used during slider drag for live updates without re-publishing.
    func sendUserVolumeToDevice(_ db: Float) {
        var val = Self.clampUserVolume(db)
        if let core = typedCore {   // -- WP SW9 --
            core.write(lane: "vol.user") { [val] in try $0.setUserVolume(db: val) }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_USER_VOLUME, value: 0, index: 0, data: data)
    }

    /// Set user volume — publish locally + send to device.  Range is
    /// clamped to the firmware-defined [-60, 0] dB window.
    func setUserVolume(_ db: Float) {
        var val = Self.clampUserVolume(db)
        self.userVolumeDB = val
        if let core = typedCore {   // -- WP SW9 --
            core.commit(lane: "vol.user") { [val] in try $0.setUserVolume(db: val) }   // -- fixup sw9 F2 --
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_USER_VOLUME, value: 0, index: 0, data: data)
    }

    func fetchUserVolume() {
        if let d = transport.getControlRequest(request: REQ_GET_USER_VOLUME, value: 0, index: 0, length: 4) {
            let val = d.withUnsafeBytes { $0.load(as: Float.self) }
            DispatchQueue.main.async {
                if abs(self.userVolumeDB - val) > 0.01 {
                    self.userVolumeDB = val
                }
            }
        }
    }

    // MARK: - LG Sound Sync

    /// Set the user-facing enable flag.  Per-preset; saved with REQ_SAVE_PRESET.
    /// Firmware emits a PARAM_CHANGED notification on the lg_sound_sync.enabled
    /// field at the V16 bulk offset (4728) — applyNotifiedParamChange mirrors that back
    /// for non-HOST sources, so other hosts' edits flow through automatically.
    func setLgSoundSyncEnabled(_ enabled: Bool) {
        DispatchQueue.main.async { self.lgSoundSyncEnabled = enabled }
        if typedWrite({ try $0.setLgSoundSync(enabled: enabled) }) { return }   // -- WP SW11 --
        transport.sendControlRequest(request: REQ_SET_LG_SOUND_SYNC_ENABLE, value: 0, index: 0,
                               data: Data([enabled ? 1 : 0]))
    }

    // MARK: - DAC Hardware Mute

    /// Write the full 16-byte DAC hardware mute config to the firmware.
    /// Optimistic local publish — the firmware emits a PARAM_CHANGED on the
    /// `dac_hw_mute` section in WireBulkParams which `applyNotifiedParamChange`
    /// will pick up for non-HOST sources, keeping multi-host clients in sync.
    ///
    /// The firmware applies/validates this deferred on its main loop and can
    /// reject it silently (e.g. an invalid or already-claimed pin) without
    /// emitting a change notification. Read it back after a short settle delay
    /// so our published value reflects what the device actually accepted rather
    /// than the optimistic guess. (Firmware contract: confirm via GET 0xEB.)
    func setDacHwMuteConfig(_ config: DacHwMuteConfig) {
        DispatchQueue.main.async { self.dacHwMuteConfig = config }
        // -- WP SW11 -- Through a core, its setter; the read-back below is its re-read.
        if !typedWrite({ try $0.setDacMute(enabled: config.enabled, activeLow: config.activeLow, pin: config.pin,
                                           holdMs: config.holdMs, releaseMs: config.releaseMs) }) {
            transport.sendControlRequest(request: REQ_SET_DAC_HW_MUTE_CONFIG,
                                   value: 0, index: 0, data: config.toData())
        }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.15) {
            self.fetchDacHwMuteConfig()
        }
    }

    /// Read the current DAC hardware mute config.  STALLs on firmware older
    /// than V10; if the call returns nil, `dacHwMuteSupported` stays false
    /// and the Settings section hides itself.
    func fetchDacHwMuteConfig() {
        // -- WP SW11 -- Through a core, its re-read into its mirror.
        if let core = typedCore {
            let generation = transport.generation
            guard case .success = core.call({ try $0.fetchDacMute() }) else { return }
            let s = core.readState()
            let cfg = DacHwMuteConfig(enabled: s.dacMuteEnabled, activeLow: s.dacMuteActiveLow, pin: s.dacMutePin,
                                      holdMs: s.dacMuteHoldMs, releaseMs: s.dacMuteReleaseMs)
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.dacHwMuteConfig != cfg { self.dacHwMuteConfig = cfg }
                self.dacHwMuteSupported = true
            }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_DAC_HW_MUTE_CONFIG,
                                            value: 0, index: 0, length: 16),
              let cfg = DacHwMuteConfig.fromData(d) else { return }
        DispatchQueue.main.async {
            self.dacHwMuteConfig = cfg
            self.dacHwMuteSupported = true
        }
    }

    /// Pulse the DAC mute line for ~1 s so the installer can audibly verify
    /// the pin / polarity wiring.  IN-direction vendor request — returns
    /// a 1-byte status (PIN_CONFIG_SUCCESS on success, PIN_CONFIG_INVALID_OUTPUT
    /// when the feature is disabled).  The audible pulse runs deferred on
    /// the firmware's main loop after the status returns.
    @discardableResult
    func testDacHwMute() -> UInt8 {
        if let core = typedCore { return Self.pinStatus(core.call { try $0.testDacMute() }) }   // -- WP SW11 --
        guard let d = transport.getControlRequest(request: REQ_TEST_DAC_HW_MUTE,
                                            value: 0, index: 0, length: 1),
              d.count >= 1 else {
            return 0xFF
        }
        return d[0]
    }

    /// Probe the firmware for LG Sound Sync support and read the current
    /// enable flag.  STALLs on firmware older than V8.
    func fetchLgSoundSyncEnabled() {
        // -- WP SW11 -- Through a core, its mirror holds the flag; the probe
        // is the core's feature list.
        if let core = typedCore {
            let supported = coreFeature(core, "lg_sound_sync")
            DispatchQueue.main.async { if self.lgSoundSyncSupported != supported { self.lgSoundSyncSupported = supported } }
            return
        }
        if let d = transport.getControlRequest(request: REQ_GET_LG_SOUND_SYNC_ENABLE, value: 0, index: 0, length: 1),
           d.count >= 1 {
            let en = d[0] != 0
            DispatchQueue.main.async {
                self.lgSoundSyncEnabled = en
                self.lgSoundSyncSupported = true
            }
        }
    }

    private static func clampUserVolume(_ db: Float) -> Float {
        let clamped = max(USER_VOLUME_MIN_DB, min(USER_VOLUME_MAX_DB, db))
        return clamped == -0.0 ? 0.0 : clamped
    }

    // MARK: - Output Latency

    /// Read the latency status and option table (REQ_OUTPUT_LATENCY wValue 0
    /// and 1).  A STALL on the status read means the firmware predates the
    /// setting, which hides the Settings section.
    func fetchOutputLatency() {
        if let core = typedCore { fetchOutputLatency(core); return }   // -- WP SW9 --
        guard let d = transport.getControlRequest(request: REQ_OUTPUT_LATENCY, value: OUTPUT_LATENCY_GET_STATUS,
                                                  index: 2, length: UInt16(OUTPUT_LATENCY_STATUS_SIZE)),
              let status = OutputLatencyStatus.fromData(d) else {
            DispatchQueue.main.async { self.outputLatencySupported = false }
            return
        }
        let tableLen = UInt16(Int(status.optionCount) * OUTPUT_LATENCY_OPTION_SIZE)
        let options = transport.getControlRequest(request: REQ_OUTPUT_LATENCY, value: OUTPUT_LATENCY_GET_OPTIONS,
                                                  index: 2, length: tableLen)
            .map(OutputLatencyOption.tableFromData)
        DispatchQueue.main.async {
            self.outputLatencyStatus = status
            if let options { self.outputLatencyOptions = options }
            self.outputLatencySupported = true
        }
    }

    /// Select a latency option.  The firmware stores it and restarts the outputs
    /// on its main loop inside a flash write, so the status reports `pending`
    /// until then; poll for that (reads can fail during the flash window) and
    /// re-read the table, whose effective values follow the new option.  Blocks:
    /// call off the main thread.
    func setOutputLatency(_ id: UInt8) {
        if let core = typedCore { setOutputLatency(id, core); return }   // -- WP SW9 --
        let generation = transport.generation
        transport.sendControlRequest(request: REQ_OUTPUT_LATENCY, value: 0, index: 2, data: Data([id]))
        for _ in 0..<40 {
            Thread.sleep(forTimeInterval: 0.1)
            guard transport.generation == generation else { return }
            if let d = transport.getControlRequest(request: REQ_OUTPUT_LATENCY, value: OUTPUT_LATENCY_GET_STATUS,
                                                   index: 2, length: UInt16(OUTPUT_LATENCY_STATUS_SIZE)),
               let st = OutputLatencyStatus.fromData(d), !st.pending { break }
        }
        fetchOutputLatency()
    }

    // -- WP SW9 -- The same two through the core's typed calls.

    private func fetchOutputLatency(_ core: CoreStateSource) {
        guard case .success(let reply) = core.call({ try $0.fetchOutputLatency() }),
              let status = OutputLatencyStatus.fromData(reply.raw) else {
            DispatchQueue.main.async { self.outputLatencySupported = false }
            return
        }
        let options = try? core.call { try $0.fetchOutputLatencyOptions() }.get().map(OutputLatencyOption.init(core:))
        DispatchQueue.main.async {
            self.outputLatencyStatus = status
            if let options { self.outputLatencyOptions = options }
            self.outputLatencySupported = true
        }
    }

    private func setOutputLatency(_ id: UInt8, _ core: CoreStateSource) {
        let generation = transport.generation
        guard case .success = core.call({ try $0.setOutputLatency(id: id) }) else { return }
        for _ in 0..<40 {
            Thread.sleep(forTimeInterval: 0.1)
            guard transport.generation == generation else { return }
            if case .success(let reply) = core.call({ try $0.fetchOutputLatency() }),
               let st = OutputLatencyStatus.fromData(reply.raw), !st.pending { break }
        }
        fetchOutputLatency(core)
    }

    // MARK: - Master Volume Mode (preset directory flag)

    /// Set master-volume persistence mode.
    /// Pass `MASTER_VOLUME_MODE_INDEPENDENT` (0, default) for the device-wide
    /// stored value or `MASTER_VOLUME_MODE_WITH_PRESET` (1) to make master
    /// volume part of each preset.
    func setMasterVolumeMode(_ mode: Int) {
        let clamped = UInt8(max(0, min(1, mode)))
        let normalized = (Int(clamped) == MASTER_VOLUME_MODE_WITH_PRESET) ? MASTER_VOLUME_MODE_WITH_PRESET
                                                                          : MASTER_VOLUME_MODE_INDEPENDENT
        DispatchQueue.main.async { self.presetMasterVolumeMode = normalized }
        transport.sendControlRequest(request: REQ_SET_MASTER_VOLUME_MODE, value: 0, index: 2, data: Data([clamped]))
    }

    func fetchMasterVolumeMode() {
        guard let data = transport.getControlRequest(request: REQ_GET_MASTER_VOLUME_MODE, value: 0, index: 2, length: 1) else { return }
        let val = Int(data[0])
        DispatchQueue.main.async {
            self.presetMasterVolumeMode = (val == MASTER_VOLUME_MODE_WITH_PRESET) ? MASTER_VOLUME_MODE_WITH_PRESET
                                                                                  : MASTER_VOLUME_MODE_INDEPENDENT
        }
    }

    /// Persist the current live master volume into the directory's independent
    /// storage. Accepted in any mode (dormant in MASTER_VOLUME_MODE_WITH_PRESET).
    /// IN-shaped action command: device responds with a 1-byte status (0 = OK).
    /// Returns true on success, false if the device disconnected or the
    /// transfer failed.
    @discardableResult
    func saveMasterVolume() -> Bool {
        guard let data = transport.getControlRequest(request: REQ_SAVE_MASTER_VOLUME, value: 0, index: 2, length: 1) else {
            return false
        }
        return data.first == 0  // PRESET_OK
    }

    /// Read the directory's independent master-volume value (the dB applied at
    /// boot in MASTER_VOLUME_MODE_INDEPENDENT). Independent of the current
    /// live value and the current mode. Returns nil if the transfer failed.
    func fetchSavedMasterVolume() -> Float? {
        guard let data = transport.getControlRequest(request: REQ_GET_SAVED_MASTER_VOLUME, value: 0, index: 2, length: 4),
              data.count >= 4 else { return nil }
        return data.withUnsafeBytes { $0.load(as: Float.self) }
    }

    func setBypass(_ enabled: Bool) {
        self.bypass = enabled
        if let core = typedCore {   // -- WP SW9 --
            core.write(lane: nil) { try $0.setBypass(enabled: enabled) }
        } else {
            var val: UInt8 = enabled ? 1 : 0
            let data = Data(bytes: &val, count: 1)
            transport.sendControlRequest(request: REQ_SET_BYPASS, value: 0, index: 0, data: data)
        }
        recomputeMagnitudes(for: 0)
        recomputeMagnitudes(for: 1)
    }
    
    /// Copy every PEQ band and the input trim from one input onto the other.
    /// Linking a pair only mirrors future edits, so this is how a mismatched
    /// pair is reconciled at the moment it's linked.  `setFilter` carries the
    /// per-band bypass flag, so bypassed bands travel too.  Call before setting
    /// the link bit: the writes are one-way and must not mirror back.
    func syncInputPair(from source: Int, to dest: Int) {
        for (band, p) in (channelData[source] ?? []).enumerated() {
            setFilter(ch: dest, band: band, p: p)
        }
        setPreampChannel(channel: dest, db: preampValue(source))
    }

    /// Reset all 10 PEQ bands of a single channel to flat.
    func clearChannelPEQ(_ ch: Int) {
        let defaultFilter = FilterParams(type: .flat, freq: 1000, q: 0.707, gain: 0)
        for b in 0..<10 {
            setFilter(ch: ch, band: b, p: defaultFilter)
        }
        recomputeMagnitudes(for: ch)
    }

    // MARK: - Loudness Compensation

    // -- WP SW10 -- Through a core, the effects' setters are its typed calls,
    // and their drag-time sends go through SW9's write lane under the
    // setting's registry path.  The committed value of a setting a drag sends
    // live goes through `commit(lane:)` with that same key (fixup sw10 F1, as
    // fixup sw9 F2), so a lane value issued after it is sent after it; a
    // discrete setting (an enable, a mask, a mode) has no lane and is a plain
    // write.  The core clamps as the firmware does, follows the limiter's link
    // groups and the tube's type rows, and journals each change, so Undo steps
    // it back.  Without a core (USB without -CoreTransport, MIDI, the Link
    // client) they stay raw SETs.

    /// -- WP SW10 -- Through a core, `body` as a typed write (in `lane` when
    /// given, or as the commit of `commit`'s lane) and true; false without
    /// one, for the raw SET.
    private func typedWrite(lane: String? = nil, commit: String? = nil,
                            _ body: @escaping (DspiCore) throws -> Void) -> Bool {
        guard let core = typedCore else { return false }
        if let key = commit {
            core.commit(lane: key, body)
        } else {
            core.write(lane: lane, body)
        }
        return true
    }

    /// -- WP SW10 -- fixup sw10 F1: the committed value of a float setting a
    /// drag sends live as `request` (`typedFloatSet`), behind its lane.
    private func typedCommit(_ request: UInt8, _ body: @escaping (DspiCore) throws -> Void) -> Bool {
        typedWrite(commit: Self.typedFloatSet(request)?.lane, body)
    }

    /// -- WP SW10 -- fixup sw10 F1: a parameter of an indexed SET (tube,
    /// limiter, upmixer) under its own key: in the lane for a drag-time send
    /// (`live`), else as the commit that supersedes it.
    private func typedKeyed(_ key: String, live: Bool, _ body: @escaping (DspiCore) throws -> Void) -> Bool {
        live ? typedWrite(lane: key, body) : typedWrite(commit: key, body)
    }

    func setLoudness(_ enabled: Bool) {
        self.loudnessEnabled = enabled
        if typedWrite({ try $0.setLoudness(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_LOUDNESS, value: 0, index: 0, data: data)
    }

    func setLoudnessRef(_ spl: Float) {
        self.loudness.refSPL = spl
        if typedCommit(REQ_SET_LOUDNESS_REF, { try $0.setLoudnessRef(spl: spl) }) { return }
        var val = spl
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_LOUDNESS_REF, value: 0, index: 0, data: data)
    }

    func setLoudnessIntensity(_ pct: Float) {
        self.loudness.intensity = pct
        if typedCommit(REQ_SET_LOUDNESS_INTENSITY, { try $0.setLoudnessIntensity(pct: pct) }) { return }
        var val = pct
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_LOUDNESS_INTENSITY, value: 0, index: 0, data: data)
    }

    /// Sets the per-output loudness mask (V19+).  Bit k enables compensation on
    /// output channel k.  Sent as a 2-byte little-endian uint16; the firmware
    /// switches masks glitch-free from the next packet (no recompute or reset).
    func setLoudnessMask(_ mask: UInt16) {
        self.loudness.outputMask = mask
        if typedWrite({ try $0.setLoudnessMask(mask: mask) }) { return }
        let data = Data([UInt8(mask & 0xFF), UInt8(mask >> 8)])
        transport.sendControlRequest(request: REQ_SET_LOUDNESS_MASK, value: 0, index: 0, data: data)
    }

    /// Toggles a single output channel's bit in the loudness mask and pushes it.
    func setLoudnessOutputChannel(_ output: Int, enabled: Bool) {
        guard output >= 0, output < 16 else { return }
        var mask = loudness.outputMask
        if enabled { mask |= (UInt16(1) << output) } else { mask &= ~(UInt16(1) << output) }
        setLoudnessMask(mask)
    }

    // MARK: - Headphone Crossfeed

    func setCrossfeed(_ enabled: Bool) {
        self.crossfeedEnabled = enabled
        if typedWrite({ try $0.setCrossfeed(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_CROSSFEED, value: 0, index: 0, data: data)
    }

    private static let presetValues: [(freq: Float, feed: Float)] = [
        (700, 4.5),   // Default
        (700, 6.0),   // Chu Moy
        (650, 9.5),   // Jan Meier
    ]

    func setCrossfeedPreset(_ preset: Int) {
        self.crossfeed.preset = preset
        if !typedWrite({ try $0.setCrossfeedPreset(preset: UInt8(clamping: preset)) }) {
            var val = UInt8(preset)
            let data = Data(bytes: &val, count: 1)
            transport.sendControlRequest(request: REQ_SET_CROSSFEED_PRESET, value: 0, index: 0, data: data)
        }
        // Apply known preset values locally so the graph updates immediately
        if preset < DSPViewModel.presetValues.count {
            self.crossfeed.freq = DSPViewModel.presetValues[preset].freq
            self.crossfeed.feed = DSPViewModel.presetValues[preset].feed
        }
    }

    func setCrossfeedFreq(_ freq: Float) {
        self.crossfeed.freq = freq
        if typedCommit(REQ_SET_CROSSFEED_FREQ, { try $0.setCrossfeedFreq(freq: freq) }) { return }
        var val = freq
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_CROSSFEED_FREQ, value: 0, index: 0, data: data)
    }

    func setCrossfeedFeed(_ feed: Float) {
        self.crossfeed.feed = feed
        if typedCommit(REQ_SET_CROSSFEED_FEED, { try $0.setCrossfeedFeed(feed: feed) }) { return }
        var val = feed
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_CROSSFEED_FEED, value: 0, index: 0, data: data)
    }

    func setCrossfeedITD(_ enabled: Bool) {
        self.crossfeed.itd = enabled
        if typedWrite({ try $0.setCrossfeedItd(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_CROSSFEED_ITD, value: 0, index: 0, data: data)
    }

    /// Sets the crossfeed output-pair mask (V20+).  Bit p runs crossfeed on output
    /// pair p (outputs 2p / 2p+1).  Sent as a single byte; the firmware clamps it to
    /// the platform's valid pair bits and switches masks glitch-free (no recompute).
    func setCrossfeedMask(_ mask: UInt8) {
        self.crossfeed.outputMask = mask
        if typedWrite({ try $0.setCrossfeedOutputs(pairMask: mask) }) { return }
        var val = mask
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_CROSSFEED_OUTPUTS, value: 0, index: 0, data: data)
    }

    /// Toggles a single output pair's bit in the crossfeed mask and pushes it.
    func setCrossfeedOutputPair(_ pair: Int, enabled: Bool) {
        guard pair >= 0, pair < 8 else { return }
        var mask = crossfeed.outputMask
        if enabled { mask |= (UInt8(1) << pair) } else { mask &= ~(UInt8(1) << pair) }
        setCrossfeedMask(mask)
    }

    // MARK: - Psychoacoustic Bass

    /// Device-only live send for a drag on any plain float parameter, the
    /// counterpart to `sendOutputGainToDevice` for the tool modules.  It
    /// transmits without publishing, so the gesture re-renders nothing; the
    /// matching `set*` commits once on release.  Clamping is the caller's, and
    /// the firmware clamps again regardless.
    /// -- WP SW10 -- Through a core, the request's typed setter in the lane.
    func sendFloatParamToDevice(_ request: UInt8, _ value: Float,
                                wValue: UInt16 = 0, wIndex: UInt16 = 0) {
        if typedCore != nil, let typed = Self.typedFloatSet(request) {
            _ = typedWrite(lane: typed.lane) { try typed.set($0, value) }
            return
        }
        var v = value
        transport.sendControlRequest(request: request, value: wValue, index: wIndex,
                                     data: Data(bytes: &v, count: 4))
    }

    /// -- WP SW10 -- The typed setter of a float SET a drag sends live, and
    /// its lane key (the registry path).  nil for a request no view sends
    /// live, which then goes raw.
    static func typedFloatSet(_ request: UInt8) -> (lane: String, set: (DspiCore, Float) throws -> Void)? {
        func bass(_ id: UInt8) -> (DspiCore, Float) throws -> Void { { try $0.setPsybassParam(param: id, value: $1) } }
        func sub(_ id: UInt8) -> (DspiCore, Float) throws -> Void { { try $0.setSubharmParam(id: id, value: $1) } }
        switch request {
        case REQ_SET_LOUDNESS_REF: return ("loud.ref", { try $0.setLoudnessRef(spl: $1) })
        case REQ_SET_LOUDNESS_INTENSITY: return ("loud.intensity", { try $0.setLoudnessIntensity(pct: $1) })
        case REQ_SET_CROSSFEED_FREQ: return ("cf.freq", { try $0.setCrossfeedFreq(freq: $1) })
        case REQ_SET_CROSSFEED_FEED: return ("cf.feed", { try $0.setCrossfeedFeed(feed: $1) })
        case REQ_SET_PSYBASS_CUTOFF: return ("bass.cutoff", bass(DspiConstants.PSYBASS_PARAM_CUTOFF))
        case REQ_SET_PSYBASS_HARMONICS: return ("bass.harmonics", bass(DspiConstants.PSYBASS_PARAM_HARMONICS))
        case REQ_SET_PSYBASS_DRIVE: return ("bass.drive", bass(DspiConstants.PSYBASS_PARAM_DRIVE))
        case REQ_SET_PSYBASS_CHARACTER: return ("bass.character", bass(DspiConstants.PSYBASS_PARAM_CHARACTER))
        case REQ_SET_PSYBASS_ORIGINAL: return ("bass.original", bass(DspiConstants.PSYBASS_PARAM_ORIGINAL))
        case REQ_SET_SUBHARM_LOW: return ("sub.low", sub(DspiConstants.SUBHARM_PARAM_LOW))
        case REQ_SET_SUBHARM_HIGH: return ("sub.high", sub(DspiConstants.SUBHARM_PARAM_HIGH))
        case REQ_SET_SUBHARM_TOP: return ("sub.top", sub(DspiConstants.SUBHARM_PARAM_TOP))
        case REQ_SET_SUBHARM_BOOST: return ("sub.boost", sub(DspiConstants.SUBHARM_PARAM_BOOST))
        case REQ_SET_SUBHARM_DEPTH: return ("sub.depth", sub(DspiConstants.SUBHARM_PARAM_DEPTH))
        case REQ_SET_SUBHARM_HOLD: return ("sub.hold", sub(DspiConstants.SUBHARM_PARAM_HOLD))
        case REQ_SET_SUBHARM_CEILING: return ("sub.ceiling", sub(DspiConstants.SUBHARM_PARAM_CEILING))
        case REQ_SET_LEVELLER_AMOUNT: return ("lev.amount", { try $0.setLevellerAmount(pct: $1) })
        case REQ_SET_LEVELLER_MAXGAIN: return ("lev.maxgain", { try $0.setLevellerMaxGain(db: $1) })
        case REQ_SET_LEVELLER_GATE: return ("lev.gate", { try $0.setLevellerGate(db: $1) })
        default: return nil
        }
    }

    func setPsybass(_ enabled: Bool) {
        self.psybassEnabled = enabled
        if typedWrite({ try $0.setPsybassEnabled(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_PSYBASS, value: 0, index: 0, data: data)
    }

    func setPsybassCutoff(_ hz: Float) {
        self.psybass.cutoffHz = hz
        if typedCommit(REQ_SET_PSYBASS_CUTOFF, { try $0.setPsybassParam(param: DspiConstants.PSYBASS_PARAM_CUTOFF, value: hz) }) { return }
        var val = hz
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PSYBASS_CUTOFF, value: 0, index: 0, data: data)
    }

    func setPsybassHarmonics(_ db: Float) {
        self.psybass.harmonicsDB = db
        if typedCommit(REQ_SET_PSYBASS_HARMONICS, { try $0.setPsybassParam(param: DspiConstants.PSYBASS_PARAM_HARMONICS, value: db) }) { return }
        var val = db
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PSYBASS_HARMONICS, value: 0, index: 0, data: data)
    }

    func setPsybassDrive(_ db: Float) {
        self.psybass.driveDB = db
        if typedCommit(REQ_SET_PSYBASS_DRIVE, { try $0.setPsybassParam(param: DspiConstants.PSYBASS_PARAM_DRIVE, value: db) }) { return }
        var val = db
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PSYBASS_DRIVE, value: 0, index: 0, data: data)
    }

    func setPsybassCharacter(_ pct: Float) {
        self.psybass.characterPct = pct
        if typedCommit(REQ_SET_PSYBASS_CHARACTER, { try $0.setPsybassParam(param: DspiConstants.PSYBASS_PARAM_CHARACTER, value: pct) }) { return }
        var val = pct
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PSYBASS_CHARACTER, value: 0, index: 0, data: data)
    }

    func setPsybassOriginal(_ db: Float) {
        self.psybass.originalDB = db
        if typedCommit(REQ_SET_PSYBASS_ORIGINAL, { try $0.setPsybassParam(param: DspiConstants.PSYBASS_PARAM_ORIGINAL, value: db) }) { return }
        var val = db
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_PSYBASS_ORIGINAL, value: 0, index: 0, data: data)
    }

    /// Sets the per-output psybass mask.  Bit k processes output channel k; sent
    /// as a 2-byte little-endian uint16.  The firmware switches masks glitch-free
    /// from the next packet (no recompute) and clears skipped outputs' state.
    func setPsybassMask(_ mask: UInt16) {
        self.psybass.outputMask = mask
        if typedWrite({ try $0.setPsybassMask(mask: mask) }) { return }
        let data = Data([UInt8(mask & 0xFF), UInt8(mask >> 8)])
        transport.sendControlRequest(request: REQ_SET_PSYBASS_MASK, value: 0, index: 0, data: data)
    }

    /// Toggles a single output channel's bit in the psybass mask and pushes it.
    func setPsybassOutputChannel(_ output: Int, enabled: Bool) {
        guard output >= 0, output < 16 else { return }
        var mask = psybass.outputMask
        if enabled { mask |= (UInt16(1) << output) } else { mask &= ~(UInt16(1) << output) }
        setPsybassMask(mask)
    }

    // MARK: - Subharmonic Synthesizer

    // Every SET is fire-and-forget and updates app state first, exactly like
    // psybass.  What differs is the headroom: the effect can add gain, and the
    // firmware recomputes an exact worst-case figure on every 0x1A GET, so each
    // SET is followed by a re-read rather than an app-side estimate.

    func setSubharm(_ enabled: Bool) {
        self.subharmEnabled = enabled
        if !typedWrite({ try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_ENABLED, value: enabled ? 1 : 0) }) {
            var val: UInt8 = enabled ? 1 : 0
            let data = Data(bytes: &val, count: 1)
            transport.sendControlRequest(request: REQ_SET_SUBHARM, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmLow(_ db: Float) {
        self.subharm.lowDB = db
        if !typedCommit(REQ_SET_SUBHARM_LOW, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_LOW, value: db) }) {
            var val = db
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_LOW, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmHigh(_ db: Float) {
        self.subharm.highDB = db
        if !typedCommit(REQ_SET_SUBHARM_HIGH, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_HIGH, value: db) }) {
            var val = db
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_HIGH, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmTop(_ db: Float) {
        self.subharm.topDB = db
        if !typedCommit(REQ_SET_SUBHARM_TOP, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_TOP, value: db) }) {
            var val = db
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_TOP, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmBoost(_ db: Float) {
        self.subharm.boostDB = db
        if !typedCommit(REQ_SET_SUBHARM_BOOST, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_BOOST, value: db) }) {
            var val = db
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_BOOST, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    /// Sets the per-output subharm mask.  Bit k processes output channel k, sent
    /// as a 2-byte little-endian uint16.  The firmware switches masks from the
    /// next audio packet with no coefficient recompute, clearing skipped
    /// outputs' state so re-enabling one is transient-free.  The headroom figure
    /// does not depend on the mask, so this SET needs no re-read.
    func setSubharmMask(_ mask: UInt16) {
        self.subharm.outputMask = mask
        if typedWrite({ try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_MASK, value: Float(mask)) }) { return }
        let data = Data([UInt8(mask & 0xFF), UInt8(mask >> 8)])
        transport.sendControlRequest(request: REQ_SET_SUBHARM_MASK, value: 0, index: 0, data: data)
    }

    /// Toggles a single output channel's bit in the subharm mask and pushes it.
    func setSubharmOutputChannel(_ output: Int, enabled: Bool) {
        guard output >= 0, output < 16 else { return }
        var mask = subharm.outputMask
        if enabled { mask |= (UInt16(1) << output) } else { mask &= ~(UInt16(1) << output) }
        setSubharmMask(mask)
    }

    /// Reads the worst-case gain of the live configuration (0 dB while disabled).
    /// Computed by the firmware on each GET, so it can never lag a coefficient
    /// recompute that a SET just triggered.
    /// -- WP SW10 -- Through a core, its `fetchSubharmHeadroom`, which runs
    /// behind the writes queued before it.
    func fetchSubharmHeadroom() {
        let val: Float
        if let core = typedCore {
            guard case .success(let v) = core.call({ try $0.fetchSubharmHeadroom() }) else { return }
            val = v
        } else {
            guard let d = transport.getControlRequest(request: REQ_GET_SUBHARM_HEADROOM, value: 0, index: 0, length: 4),
                  d.count >= 4 else { return }
            val = d.withUnsafeBytes { $0.load(as: Float.self) }
        }
        DispatchQueue.main.async {
            if abs(self.subharm.headroomDB - val) > 0.001 { self.subharm.headroomDB = val }
        }
    }

    /// Re-reads the headroom off the main thread.  Called after every SET that
    /// can change it; the USB serial queue orders the GET behind the SET, so the
    /// figure always reflects the value just written.
    private func fetchSubharmHeadroomAsync() {
        guard firmwareSupportsSubharm else { return }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.fetchSubharmHeadroom()
        }
    }

    // MARK: Selectivity, ceiling, pair link, solo and meter (V30)

    /// Selectivity mode (0 all, 1 percussive, 2 sustained).  The firmware clamps
    /// anything above 2, so the app clamps to the same range rather than relying
    /// on a read-back.
    func setSubharmSelectMode(_ mode: Int) {
        let clamped = min(max(mode, SUBHARM_SELECT_ALL), SUBHARM_SELECT_SUSTAINED)
        self.subharm.selectMode = clamped
        if !typedWrite({ try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_SELECT, value: Float(clamped)) }) {
            var val = UInt8(clamped)
            let data = Data(bytes: &val, count: 1)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_SELECT, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmSelectDepth(_ pct: Float) {
        self.subharm.selectDepthPct = pct
        if !typedCommit(REQ_SET_SUBHARM_DEPTH, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_DEPTH, value: pct) }) {
            var val = pct
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_DEPTH, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    func setSubharmSelectHold(_ ms: Float) {
        self.subharm.selectHoldMs = ms
        if !typedCommit(REQ_SET_SUBHARM_HOLD, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_HOLD, value: ms) }) {
            var val = ms
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_HOLD, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    /// Sub ceiling in dBFS; 0 turns the limiter off.  It bounds the sub before it
    /// is mixed in, so the firmware's headroom figure drops to whatever the
    /// ceiling allows - which is why this SET re-reads it like a level does.
    func setSubharmCeiling(_ dbfs: Float) {
        self.subharm.ceilingDB = dbfs
        if !typedCommit(REQ_SET_SUBHARM_CEILING, { try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_CEILING, value: dbfs) }) {
            var val = dbfs
            let data = Data(bytes: &val, count: 4)
            transport.sendControlRequest(request: REQ_SET_SUBHARM_CEILING, value: 0, index: 0, data: data)
        }
        fetchSubharmHeadroomAsync()
    }

    /// Link each output pair to its mono sum.  Read live each packet by the
    /// firmware, so there is no recompute and the headroom cannot change: a
    /// mono sum is never larger than the louder of the two channels.
    func setSubharmLinkPairs(_ linked: Bool) {
        self.subharm.linkPairs = linked
        if typedWrite({ try $0.setSubharmParam(id: DspiConstants.SUBHARM_PARAM_LINK, value: linked ? 1 : 0) }) { return }
        var val: UInt8 = linked ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_SUBHARM_LINK, value: 0, index: 0, data: data)
    }

    /// Solo the synthesized sub on the masked outputs.  Runtime only: it is not
    /// on the wire and not in any preset, so the app has to read it back with
    /// 0x2D rather than expecting it from `fetchAllParams`, and clears it when
    /// the window closes so a device is never left playing without program.
    func setSubharmSolo(_ solo: Bool) {
        self.subharm.solo = solo
        if typedWrite({ try $0.setSubharmSolo(solo: solo) }) { return }
        var val: UInt8 = solo ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_SUBHARM_SOLO, value: 0, index: 0, data: data)
    }

    /// -- WP SW10 -- Through a core, the core's copy (SWIFT-SEAMS 3: a field
    /// read): read on connect and kept by this app's writes.  The firmware
    /// never notifies solo, so another Link client's change shows after the
    /// next full read.
    func fetchSubharmSolo() {
        if let core = typedCore {
            let val = core.readState().subharmSolo
            DispatchQueue.main.async { self.subharm.solo = val }
            return
        }
        if let d = transport.getControlRequest(request: REQ_GET_SUBHARM_SOLO, value: 0, index: 0, length: 1), !d.isEmpty {
            let val = d[0] != 0
            DispatchQueue.main.async { self.subharm.solo = val }
        }
    }

    /// Reads the per-output sub meter: one uint16 LE per output channel on the
    /// same 0..32767 scale as the status peaks, normalized here so it drives the
    /// same meter widget.  A short read is kept rather than discarded, since the
    /// entries that did arrive are still valid.
    func fetchSubharmMeter() {
        let want = numOutputChannels
        guard want > 0 else { return }
        if let core = typedCore {   // -- WP SW10 --
            guard case .success(let all) = core.call({ try $0.fetchSubharmMeter() }) else { return }
            let levels = Array(all.prefix(want))
            DispatchQueue.main.async { self.subharm.meter.levels = levels }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_SUBHARM_METER, value: 0, index: 0,
                                            length: UInt16(want * 2)), d.count >= 2 else { return }
        var levels: [Float] = []
        levels.reserveCapacity(min(want, d.count / 2))
        for i in 0..<min(want, d.count / 2) {
            let raw = UInt16(d[i * 2]) | (UInt16(d[i * 2 + 1]) << 8)
            levels.append(Float(raw) / SUBHARM_METER_FULL_SCALE)
        }
        DispatchQueue.main.async { self.subharm.meter.levels = levels }
    }

    /// Reads back the two subharm values the bulk image does not carry: solo,
    /// which is runtime-only, and the headroom, which the firmware derives.
    /// Called when the subharm window opens.
    func fetchSubharmRuntimeState() {
        guard firmwareSupportsSubharm else { return }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            self.fetchSubharmHeadroom()
            if self.firmwareSupportsSubharmExtended { self.fetchSubharmSolo() }
        }
    }

    // MARK: - Tube Modeller (V31, cmds 0x3E/0x3F)

    // One indexed SET/GET pair carries all fourteen parameters as float32, the
    // bools, mask and enums included.  Every SET is fire-and-forget and updates
    // app state first; the app clamps exactly as the firmware does, and mirrors
    // the two cross-field rules (a type loads its row, a character edit drops the
    // type to Custom) because the firmware's notifications for our own writes are
    // filtered out as HOST-sourced and would never correct us.

    /// -- WP SW10 -- Through a core, `setTubeParam` (it follows the type
    /// rows and the drop to Custom too); `lane` for a drag-time send.
    private func sendTubeParam(_ index: UInt16, _ value: Float, lane: Bool = false) {
        if typedKeyed("tube[\(index)]", live: lane, { try $0.setTubeParam(idx: UInt8(clamping: index), value: value) }) {
            return
        }
        var val = value
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_TUBE_PARAM, value: index, index: 0, data: data)
    }

    private static func clampTube(_ v: Float, _ lo: Float, _ hi: Float) -> Float {
        min(max(v, lo), hi)
    }

    /// Device-only live send for a drag, mirroring `sendOutputGainToDevice`.
    /// It clamps and transmits but publishes nothing, so a gesture never
    /// re-renders the window; `ParameterRow` commits once on release through
    /// the matching `setTube*`.
    func sendTubeParamToDevice(_ index: UInt16, _ value: Float, _ lo: Float, _ hi: Float) {
        sendTubeParam(index, Self.clampTube(value, lo, hi), lane: true)
    }

    func setTube(_ enabled: Bool) {
        self.tubeEnabled = enabled
        sendTubeParam(TUBE_PARAM_ENABLED, enabled ? 1 : 0)
    }

    /// Sets the per-output mask.  The firmware reads it live each packet with no
    /// recompute, and resets a masked-off output's state so it re-enters cleanly.
    func setTubeMask(_ mask: UInt16) {
        self.tube.outputMask = mask
        sendTubeParam(TUBE_PARAM_OUTPUT_MASK, Float(mask))
    }

    func setTubeOutputChannel(_ output: Int, enabled: Bool) {
        guard output >= 0, output < 16 else { return }
        var mask = tube.outputMask
        if enabled { mask |= (UInt16(1) << output) } else { mask &= ~(UInt16(1) << output) }
        setTubeMask(mask)
    }

    /// Selects a tube style.  1..16 copies that row into the four character
    /// knobs; 0 (Custom) leaves them where they are.
    func setTubeType(_ type: Int) {
        let t = min(max(type, TUBE_TYPE_CUSTOM), TUBE_TYPE_MAX)
        self.tube.type = t
        if t < TUBE_TYPE_ROWS.count, let row = TUBE_TYPE_ROWS[t] {
            self.tube.biasPct = row.biasPct
            self.tube.asymDB = row.asymDB
            self.tube.hardnessPct = row.hardnessPct
            self.tube.sagPct = row.sagPct
        }
        sendTubeParam(TUBE_PARAM_TUBE_TYPE, Float(t))
    }

    func setTubeDrive(_ db: Float) {
        let v = Self.clampTube(db, TUBE_DRIVE_MIN, TUBE_DRIVE_MAX)
        self.tube.driveDB = v
        sendTubeParam(TUBE_PARAM_DRIVE_DB, v)
    }

    /// Shared by the four character knobs.  Only a real change drops the type
    /// to Custom, matching the firmware: re-sending the stored value keeps it.
    private func setTubeCharacter(_ index: UInt16, _ value: Float,
                                  _ lo: Float, _ hi: Float,
                                  _ field: ReferenceWritableKeyPath<TubeParameters, Float>) {
        let v = Self.clampTube(value, lo, hi)
        if tube[keyPath: field] != v {
            tube[keyPath: field] = v
            self.tube.type = TUBE_TYPE_CUSTOM
        }
        sendTubeParam(index, v)
    }

    func setTubeBias(_ pct: Float) {
        setTubeCharacter(TUBE_PARAM_BIAS_PCT, pct, TUBE_BIAS_MIN, TUBE_BIAS_MAX, \.biasPct)
    }

    func setTubeAsym(_ db: Float) {
        setTubeCharacter(TUBE_PARAM_ASYM_DB, db, TUBE_ASYM_MIN, TUBE_ASYM_MAX, \.asymDB)
    }

    func setTubeHardness(_ pct: Float) {
        setTubeCharacter(TUBE_PARAM_HARDNESS_PCT, pct, TUBE_HARDNESS_MIN, TUBE_HARDNESS_MAX, \.hardnessPct)
    }

    func setTubeSag(_ pct: Float) {
        setTubeCharacter(TUBE_PARAM_SAG_PCT, pct, TUBE_SAG_MIN, TUBE_SAG_MAX, \.sagPct)
    }

    func setTubeRectifier(_ rect: Int) {
        let r = min(max(rect, TUBE_RECT_SOLID_STATE), TUBE_RECT_MAX)
        self.tube.rectifier = r
        sendTubeParam(TUBE_PARAM_RECTIFIER, Float(r))
    }

    func setTubeXfmr(_ enabled: Bool) {
        self.tube.xfmrEnabled = enabled
        sendTubeParam(TUBE_PARAM_XFMR_ENABLED, enabled ? 1 : 0)
    }

    func setTubeXfmrDamping(_ df: Float) {
        let v = Self.clampTube(df, TUBE_XFMR_DAMPING_MIN, TUBE_XFMR_DAMPING_MAX)
        self.tube.xfmrDamping = v
        sendTubeParam(TUBE_PARAM_XFMR_DAMPING, v)
    }

    func setTubeXfmrRes(_ hz: Float) {
        let v = Self.clampTube(hz, TUBE_XFMR_RES_MIN, TUBE_XFMR_RES_MAX)
        self.tube.xfmrResHz = v
        sendTubeParam(TUBE_PARAM_XFMR_RES_HZ, v)
    }

    func setTubeMix(_ pct: Float) {
        let v = Self.clampTube(pct, TUBE_MIX_MIN, TUBE_MIX_MAX)
        self.tube.mixPct = v
        sendTubeParam(TUBE_PARAM_MIX_PCT, v)
    }

    func setTubeTrim(_ db: Float) {
        let v = Self.clampTube(db, TUBE_TRIM_MIN, TUBE_TRIM_MAX)
        self.tube.trimDB = v
        sendTubeParam(TUBE_PARAM_TRIM_DB, v)
    }

    // MARK: - Output Limiter (V32, cmd 0x81)

    // One opcode, wValue = (output << 8) | index, float32 payload for every
    // parameter.  SETs are fire-and-forget and update app state first, clamped
    // and rounded exactly as the firmware does, because notifications for our
    // own writes are filtered out and would never correct us.
    //
    // The settings follow output_config_mode like the pins, so in INDEPENDENT
    // mode an edit is part of the unsaved output configuration: user-facing
    // callers must mark it (SettingsSaveCoordinator.beginOutputEdit).
    //
    // Linked outputs are ganged by the firmware (spec §2.2): one SET reaches
    // the whole group, and a joining output adopts the group's settings.  The
    // app mirrors that through `LimiterGang` so it shows what the device holds.

    /// -- WP SW10 -- Through a core, `setLimiterParam`; `lane` for a
    /// drag-time send.  The core gangs a linked group in its mirror as the
    /// firmware does; `LimiterGang` stays for the paths without a core.
    private func sendLimiterParam(output: UInt8, _ index: UInt8, _ value: Float, lane: Bool = false) {
        if typedKeyed("limit[\(output)][\(index)]", live: lane,
                      { try $0.setLimiterParam(output: output, param: index, value: value) }) {
            return
        }
        var val = value
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_LIMITER, value: (UInt16(output) << 8) | UInt16(index),
                                     index: 0, data: data)
    }

    static func clampLimiterThreshold(_ db: Float) -> Float {
        min(max(db, LIMITER_THRESHOLD_MIN), LIMITER_THRESHOLD_MAX)
    }

    static func clampLimiterRelease(_ ms: Float) -> Float {
        min(max(ms, LIMITER_RELEASE_MIN), LIMITER_RELEASE_MAX)
    }

    /// The firmware rounds to the nearest group after clamping.
    static func clampLimiterLinkGroup(_ group: Int) -> Int {
        min(max(group, 0), LIMITER_LINK_GROUP_MAX)
    }

    private func isLimiterOutput(_ output: Int) -> Bool {
        output >= 0 && output < min(numOutputChannels, limiter.outputs.count)
    }

    /// Device-only live send for a threshold or release drag.  Publishes
    /// nothing; `ParameterRow` commits once on release through the setter.
    func sendLimiterParamToDevice(output: Int, _ index: UInt8, _ value: Float) {
        guard isLimiterOutput(output) else { return }
        let v = index == LIMITER_PARAM_RELEASE_MS ? Self.clampLimiterRelease(value)
                                                  : Self.clampLimiterThreshold(value)
        sendLimiterParam(output: UInt8(output), index, v, lane: true)
    }

    /// Mirrors an enable, threshold or release SET: it reaches the whole
    /// group of a linked output.  Published once, as a single array write.
    private func editLimiterGanged(_ output: Int, _ change: (inout LimiterOutputSettings) -> Void) {
        var outs = limiter.outputs
        LimiterGang.edit(output, in: &outs, count: numOutputChannels, change)
        limiter.outputs = outs
    }

    func setLimiterEnabled(output: Int, _ enabled: Bool) {
        guard isLimiterOutput(output) else { return }
        editLimiterGanged(output) { $0.enabled = enabled }
        sendLimiterParam(output: UInt8(output), LIMITER_PARAM_ENABLED, enabled ? 1 : 0)
    }

    func setLimiterThreshold(output: Int, _ db: Float) {
        guard isLimiterOutput(output), !db.isNaN else { return }
        let v = Self.clampLimiterThreshold(db)
        editLimiterGanged(output) { $0.thresholdDB = v }
        sendLimiterParam(output: UInt8(output), LIMITER_PARAM_THRESHOLD_DB, v)
    }

    func setLimiterRelease(output: Int, _ ms: Float) {
        guard isLimiterOutput(output), !ms.isNaN else { return }
        let v = Self.clampLimiterRelease(ms)
        editLimiterGanged(output) { $0.releaseMs = v }
        sendLimiterParam(output: UInt8(output), LIMITER_PARAM_RELEASE_MS, v)
    }

    /// Joining a group adopts the settings of its lowest-numbered existing
    /// member; the first member keeps its own, and leaving keeps the current
    /// ones.
    func setLimiterLinkGroup(output: Int, _ group: Int) {
        guard isLimiterOutput(output) else { return }
        let g = Self.clampLimiterLinkGroup(group)
        var outs = limiter.outputs
        LimiterGang.setGroup(output, g, in: &outs, count: numOutputChannels)
        limiter.outputs = outs
        sendLimiterParam(output: UInt8(output), LIMITER_PARAM_LINK_GROUP, Float(g))
    }

    /// Puts outputs back to exact per-output settings, for Revert and for a
    /// preset-file import.  Ganging would otherwise scramble a plain
    /// write-each-field pass: a threshold written to an output still in its old
    /// group would spread to that group.  So the targets leave their groups
    /// first, take their own values unlinked, then rejoin in ascending order,
    /// which makes the lowest member of each group its leader, as the firmware
    /// does.  Enable is written last so a limiter engages on its new threshold.
    func applyLimiterSettings(_ targets: [Int: LimiterOutputSettings]) {
        let outs = targets.keys.filter(isLimiterOutput).sorted()
        for k in outs where limiter.outputs[k].linkGroup != 0 {
            setLimiterLinkGroup(output: k, 0)
        }
        for k in outs {
            let t = targets[k]!
            setLimiterThreshold(output: k, t.thresholdDB)
            setLimiterRelease(output: k, t.releaseMs)
            setLimiterEnabled(output: k, t.enabled)
        }
        for k in outs where targets[k]!.linkGroup != 0 {
            setLimiterLinkGroup(output: k, targets[k]!.linkGroup)
        }
    }

    /// Writes one output's settings to every output with a single SET per
    /// parameter (output 0xFF), which is what the firmware's all-outputs form
    /// is for.  The link group is left alone: copying it would link every
    /// output into one group.  Every output gets the same values, so groups
    /// stay ganged without any mirroring.
    func copyLimiterToAllOutputs(from output: Int) {
        guard isLimiterOutput(output) else { return }
        let src = limiter.outputs[output]
        var outs = limiter.outputs
        for k in 0..<min(numOutputChannels, outs.count) {
            outs[k].enabled = src.enabled
            outs[k].thresholdDB = src.thresholdDB
            outs[k].releaseMs = src.releaseMs
        }
        limiter.outputs = outs
        sendLimiterParam(output: LIMITER_ALL_OUTPUTS, LIMITER_PARAM_THRESHOLD_DB, src.thresholdDB)
        sendLimiterParam(output: LIMITER_ALL_OUTPUTS, LIMITER_PARAM_RELEASE_MS, src.releaseMs)
        sendLimiterParam(output: LIMITER_ALL_OUTPUTS, LIMITER_PARAM_ENABLED, src.enabled ? 1 : 0)
    }

    /// Switches every output's limiter on or off with one all-outputs SET.
    func setLimiterEnabledOnAll(_ enabled: Bool) {
        guard firmwareSupportsLimiter else { return }
        var outs = limiter.outputs
        for k in 0..<min(numOutputChannels, outs.count) { outs[k].enabled = enabled }
        limiter.outputs = outs
        sendLimiterParam(output: LIMITER_ALL_OUTPUTS, LIMITER_PARAM_ENABLED, enabled ? 1 : 0)
    }

    /// Sets the link group of every output at once, one SET each, in ascending
    /// order, so each group's lowest output keeps its settings and the rest
    /// adopt them.  `groups` shorter than the output count leaves the rest
    /// unlinked.
    func setLimiterLinkGroups(_ groups: [Int]) {
        for k in 0..<min(numOutputChannels, limiter.outputs.count) {
            setLimiterLinkGroup(output: k, k < groups.count ? groups[k] : 0)
        }
    }

    /// Polls the gain-reduction meter (0x80) when `meter` is set, and clears
    /// it once otherwise.  Called from the shared poll timer, off the main
    /// thread.  Publishes only what changed.
    func pollLimiter(meter: Bool) {
        guard firmwareSupportsLimiter else { return }
        let want = numOutputChannels
        guard want > 0 else { return }
        guard meter else {
            // Nothing can be limiting; clear the bars once rather than polling.
            DispatchQueue.main.async {
                if self.limiter.meter.reductionDB.contains(where: { $0 != 0 }) {
                    self.limiter.meter.reductionDB = []
                }
            }
            return
        }
        var gr: [Float] = []
        if let core = typedCore {   // -- WP SW10 --
            guard case .success(let all) = core.call({ try $0.fetchLimiterMeter() }) else { return }
            gr = Array(all.prefix(want))
        } else {
            guard let d = transport.getControlRequest(request: REQ_LIMITER, value: UInt16(LIMITER_GET_METER),
                                                      index: 0, length: UInt16(want * 2)), d.count >= 2 else { return }
            gr.reserveCapacity(min(want, d.count / 2))
            for i in 0..<min(want, d.count / 2) {
                let raw = UInt16(d[i * 2]) | (UInt16(d[i * 2 + 1]) << 8)
                gr.append(Float(raw) / 100)
            }
        }
        DispatchQueue.main.async {
            if self.limiter.meter.reductionDB != gr { self.limiter.meter.reductionDB = gr }
        }
    }

    // MARK: - Stereo Upmixer (V25, cmds 0x4A-0x4E)

    /// Sends one upmix parameter as a 4-byte LE float via REQ_UPMIX_SET_PARAM
    /// (spec §6.2).  Preferred for live sliders: no read-modify-write race with
    /// other controllers.  The firmware clamps out-of-range values.
    /// -- WP SW10 -- Through a core, `setUpmixParam`; `lane` for a drag-time
    /// send.
    private func sendUpmixParam(_ paramId: UInt16, _ value: Float, lane: Bool = false) {
        if typedKeyed("up[\(paramId)]", live: lane,
                      { try $0.setUpmixParam(id: UInt8(clamping: paramId), value: value) }) {
            return
        }
        var v = value
        let data = Data(bytes: &v, count: 4)
        transport.sendControlRequest(request: REQ_UPMIX_SET_PARAM, value: paramId, index: 0, data: data)
    }

    func setUpmixEnabled(_ enabled: Bool) {
        self.upmixEnabled = enabled
        sendUpmixParam(UPMIX_PARAM_ENABLED, enabled ? 1.0 : 0.0)
    }

    func setUpmixCenterMode(_ mode: Int) {
        self.upmix.centerMode = mode
        sendUpmixParam(UPMIX_PARAM_CENTER_MODE, Float(mode))
    }

    func setUpmixSurroundMode(_ mode: Int) {
        self.upmix.surroundMode = mode
        sendUpmixParam(UPMIX_PARAM_SURROUND_MODE, Float(mode))
    }

    /// Device-only live send for a drag; see `sendFloatParamToDevice`.
    func sendUpmixParamToDevice(_ index: UInt16, _ value: Float) {
        sendUpmixParam(index, value, lane: true)
    }

    func setUpmixStrength(_ pct: Float) {
        self.upmix.strengthPct = pct
        sendUpmixParam(UPMIX_PARAM_STRENGTH, pct)
    }

    func setUpmixCenterWidth(_ pct: Float) {
        self.upmix.centerWidthPct = pct
        sendUpmixParam(UPMIX_PARAM_CENTER_WIDTH, pct)
    }

    func setUpmixThreshold(_ pct: Float) {
        self.upmix.thresholdPct = pct
        sendUpmixParam(UPMIX_PARAM_THRESHOLD, pct)
    }

    func setUpmixAttack(_ ms: Float) {
        self.upmix.attackMs = ms
        sendUpmixParam(UPMIX_PARAM_ATTACK, ms)
    }

    func setUpmixRelease(_ ms: Float) {
        self.upmix.releaseMs = ms
        sendUpmixParam(UPMIX_PARAM_RELEASE, ms)
    }

    func setUpmixDetectorHpf(_ hz: Float) {
        self.upmix.detectorHpfHz = hz
        sendUpmixParam(UPMIX_PARAM_DET_HPF, hz)
    }

    func setUpmixSurroundDelay(_ ms: Float) {
        self.upmix.surroundDelayMs = ms
        sendUpmixParam(UPMIX_PARAM_SUR_DELAY, ms)
    }

    func setUpmixSurroundHpf(_ hz: Float) {
        self.upmix.surroundHpfHz = hz
        sendUpmixParam(UPMIX_PARAM_SUR_HPF, hz)
    }

    func setUpmixSurroundLpf(_ hz: Float) {
        self.upmix.surroundLpfHz = hz
        sendUpmixParam(UPMIX_PARAM_SUR_LPF, hz)
    }

    func setUpmixDecorr(_ pct: Float) {
        self.upmix.decorrPct = pct
        sendUpmixParam(UPMIX_PARAM_DECORR, pct)
    }

    /// Centre presence bell gain (dB).  SET_PARAM carries a plain float dB (the
    /// firmware quantizes to 0.5 dB steps when it packs the config packet).
    func setUpmixPresence(_ db: Float) {
        self.upmix.presenceDB = db
        sendUpmixParam(UPMIX_PARAM_PRESENCE, db)
    }

    /// Reads the whole 44-byte UpmixConfigPacket in one transfer and publishes it
    /// (spec §6.1).  Called after a preset load / bulk SET; the bulk parse also
    /// keeps these fields current.  On RP2040 the GET returns zeros (feature off).
    /// -- WP SW10 -- Through a core, nothing to read: the core's mirror holds
    /// the upmixer's settings and CoreMirror publishes them.
    func fetchUpmixConfig() {
        guard typedCore == nil else { return }
        guard let d = transport.getControlRequest(request: REQ_UPMIX_GET_CONFIG, value: 0, index: 0,
                                            length: UInt16(UPMIX_CONFIG_PACKET_SIZE)),
              d.count >= UPMIX_CONFIG_PACKET_SIZE else { return }
        let enabled = d[0] != 0
        let centerMode = Int(d[1])
        let surroundMode = Int(d[2])
        // Byte 3 = presence_q1 (signed i8, 0.5 dB steps); V25 firmware wrote 0 here.
        let presence = Float(Int8(bitPattern: d[3])) / 2.0
        func f(_ off: Int) -> Float { d.withUnsafeBytes { $0.load(fromByteOffset: off, as: Float.self) } }
        let strength = f(4), width = f(8), threshold = f(12), attack = f(16), release = f(20)
        let detHpf = f(24), surDelay = f(28), surHpf = f(32), surLpf = f(36), decorr = f(40)
        DispatchQueue.main.async {
            self.upmixEnabled = enabled
            self.upmix.centerMode = centerMode
            self.upmix.surroundMode = surroundMode
            self.upmix.presenceDB = presence
            self.upmix.strengthPct = strength
            self.upmix.centerWidthPct = width
            self.upmix.thresholdPct = threshold
            self.upmix.attackMs = attack
            self.upmix.releaseMs = release
            self.upmix.detectorHpfHz = detHpf
            self.upmix.surroundDelayMs = surDelay
            self.upmix.surroundHpfHz = surHpf
            self.upmix.surroundLpfHz = surLpf
            self.upmix.decorrPct = decorr
        }
    }

    /// Polls the 16-byte UpmixStatus telemetry (spec §6.3).  Called from the
    /// shared poll timer only while the upmixer window is open.
    func fetchUpmixStatus() {
        if let core = typedCore {   // -- WP SW10 --
            guard case .success(let st) = core.call({ try $0.fetchUpmixStatus() }) else { return }
            DispatchQueue.main.async {
                self.upmix.telemetry.active = st.active
                self.upmix.telemetry.parkedReason = st.parkedReason
                self.upmix.telemetry.corr = st.correlation
                self.upmix.telemetry.balance = st.balance
                self.upmix.telemetry.centerGain = st.centerGain
                self.upmix.telemetry.lsGain = st.lsGain
                self.upmix.telemetry.rsGain = st.rsGain
            }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_UPMIX_GET_STATUS, value: 0, index: 0,
                                            length: UPMIX_STATUS_SIZE),
              d.count >= 16 else { return }
        let active = d[0] != 0
        let parked = d[1]
        let corr = d.withUnsafeBytes { $0.load(fromByteOffset: 2, as: Int16.self) }
        let balance = d.withUnsafeBytes { $0.load(fromByteOffset: 4, as: UInt16.self) }
        let centerGain = d.withUnsafeBytes { $0.load(fromByteOffset: 6, as: UInt16.self) }
        let lsGain = d.withUnsafeBytes { $0.load(fromByteOffset: 8, as: UInt16.self) }
        let rsGain = d.withUnsafeBytes { $0.load(fromByteOffset: 10, as: UInt16.self) }
        DispatchQueue.main.async {
            self.upmix.telemetry.active = active
            self.upmix.telemetry.parkedReason = parked
            self.upmix.telemetry.corr = Float(corr) / 16384.0        // Q14, [-1, +1]
            self.upmix.telemetry.balance = Float(balance) / 16384.0  // Q14, 0..1
            self.upmix.telemetry.centerGain = Float(centerGain) / 32767.0  // Q15, 0..1
            self.upmix.telemetry.lsGain = Float(lsGain) / 32767.0
            self.upmix.telemetry.rsGain = Float(rsGain) / 32767.0
        }
    }

    // MARK: - Volume Leveller

    func setLeveller(_ enabled: Bool) {
        self.levellerEnabled = enabled
        if typedWrite({ try $0.setLevellerEnabled(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_LEVELLER, value: 0, index: 0, data: data)
    }

    func setLevellerAmount(_ amount: Float) {
        self.leveller.amount = amount
        if typedCommit(REQ_SET_LEVELLER_AMOUNT, { try $0.setLevellerAmount(pct: amount) }) { return }
        var val = amount
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_LEVELLER_AMOUNT, value: 0, index: 0, data: data)
    }

    func setLevellerSpeed(_ speed: Int) {
        self.leveller.speed = speed
        if typedWrite({ try $0.setLevellerSpeed(speed: UInt8(clamping: speed)) }) { return }
        var val = UInt8(speed)
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_LEVELLER_SPEED, value: 0, index: 0, data: data)
    }

    func setLevellerMaxGain(_ db: Float) {
        self.leveller.maxGainDB = db
        if typedCommit(REQ_SET_LEVELLER_MAXGAIN, { try $0.setLevellerMaxGain(db: db) }) { return }
        var val = db
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_LEVELLER_MAXGAIN, value: 0, index: 0, data: data)
    }

    func setLevellerLookahead(_ enabled: Bool) {
        self.leveller.lookahead = enabled
        if typedWrite({ try $0.setLevellerLookahead(enabled: enabled) }) { return }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_LEVELLER_LOOKAHEAD, value: 0, index: 0, data: data)
    }

    func setLevellerGate(_ db: Float) {
        self.leveller.gateDB = db
        if typedCommit(REQ_SET_LEVELLER_GATE, { try $0.setLevellerGate(db: db) }) { return }
        var val = db
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_LEVELLER_GATE, value: 0, index: 0, data: data)
    }

    /// Sets both leveller channel masks (V18). `detector` selects which inputs
    /// feed the shared RMS detector; `apply` selects which inputs receive the
    /// shared gain. Bit k = input channel k. Sent as a single 2-byte payload;
    /// the firmware switches masks glitch-free without a state reset.
    func setLevellerMasks(detector: UInt8, apply: UInt8) {
        self.leveller.detectorMask = detector
        self.leveller.applyMask = apply
        if typedWrite({ try $0.setLevellerMasks(detector: detector, apply: apply) }) { return }
        let data = Data([detector, apply])
        transport.sendControlRequest(request: REQ_SET_LEVELLER_MASKS, value: 0, index: 0, data: data)
    }

    /// Toggles a single channel's bit in the detector mask and pushes both masks.
    func setLevellerDetectorChannel(_ channel: Int, enabled: Bool) {
        guard channel >= 0, channel < 8 else { return }
        var mask = leveller.detectorMask
        if enabled { mask |= (1 << channel) } else { mask &= ~(UInt8(1) << channel) }
        setLevellerMasks(detector: mask, apply: leveller.applyMask)
    }

    /// Toggles a single channel's bit in the apply mask and pushes both masks.
    func setLevellerApplyChannel(_ channel: Int, enabled: Bool) {
        guard channel >= 0, channel < 8 else { return }
        var mask = leveller.applyMask
        if enabled { mask |= (1 << channel) } else { mask &= ~(UInt8(1) << channel) }
        setLevellerMasks(detector: leveller.detectorMask, apply: mask)
    }

    // MARK: - Matrix Mixer

    /// -- WP SW9 -- fixup sw9 F3: a crosspoint gain as it will be kept:
    /// through a core, its 0.1 dB grid (`quantize_gain`), so the value shown
    /// is the one the device gets; else as given.
    func keptMatrixGain(_ gain: Float) -> Float {
        guard typedCore != nil else { return gain }
        let g = (gain * 10).rounded() / 10
        return g == -0.0 ? 0.0 : g
    }

    func setMatrixRoute(input: Int, output: Int, enabled: Bool, gain: Float, invert: Bool) {
        let gain = keptMatrixGain(gain)   // -- fixup sw9 F3 --
        matrixRouting[input][output] = enabled
        matrixGain[input][output] = gain
        matrixInvert[input][output] = invert

        if let core = typedCore {   // -- WP SW9 --
            core.write(lane: nil) {
                try $0.setMatrixRoute(input: Self.wire(input), output: Self.wire(output), enabled: enabled,
                                      gain: gain, invert: invert)
            }
            return
        }

        var packet = Data(count: 9)
        packet[0] = UInt8(input)
        packet[1] = UInt8(output)
        packet[2] = enabled ? 1 : 0
        packet[3] = invert ? 1 : 0
        var g = gain
        withUnsafeBytes(of: &g) { packet.replaceSubrange(4..<8, with: $0) }

        transport.sendControlRequest(request: REQ_SET_MATRIX_ROUTE, value: 0, index: 2, data: packet)
    }

    /// Apply a direct 1:1 routing (recipe 11.1 of the 8-channel-usb-input spec):
    /// input i → output i at 0 dB for i in 0..<numMatrixInputs, enabling those
    /// outputs and disabling the PDM sub.  All other crosspoints are cleared so
    /// the result is exactly the diagonal.  Only issues async (fire-and-forget)
    /// control transfers and no blocking read-backs, so it is safe to call on
    /// the main thread.
    func applyDirectRouting() {
        let n = min(numMatrixInputs, numOutputChannels)
        // Free Core 1 first: disabling the PDM sub lets the per-output EQ workers
        // (outputs 2-7) be enabled without a shared-resource conflict.
        if outputEnabled[pdmOutputIndex] {
            setOutputEnable(output: pdmOutputIndex, enabled: false)
        }
        // Reset every crosspoint to the i→i diagonal.
        for i in 0..<numMatrixInputs {
            for o in 0..<numOutputChannels {
                let on = (i == o && i < n)
                if matrixRouting[i][o] != on || matrixGain[i][o] != 0 || matrixInvert[i][o] {
                    setMatrixRoute(input: i, output: o, enabled: on, gain: 0, invert: false)
                }
            }
        }
        // Enable the diagonal outputs (PDM stays off).
        for o in 0..<n where o != pdmOutputIndex {
            if !outputEnabled[o] { setOutputEnable(output: o, enabled: true) }
        }
    }

    /// Disconnect every matrix crosspoint, leaving gains/phase untouched.  Run
    /// off the main thread.
    func clearAllRoutes() {
        for i in 0..<numMatrixInputs {
            for o in 0..<numOutputChannels where matrixRouting[i][o] {
                setMatrixRoute(input: i, output: o, enabled: false,
                               gain: matrixGain[i][o], invert: matrixInvert[i][o])
            }
        }
    }

    // MARK: - Per-Output Enable

    func setOutputEnable(output: Int, enabled: Bool) {
        outputEnabled[output] = enabled
        if isOverviewMode {
            channelVisibility[eqChannel(forOutput: output)] = enabled
        }
        if let core = typedCore {   // -- WP SW9 --
            // The core checks Core 1 first and reads the output back: a
            // refused enable is shown as the device has it.
            core.write(lane: nil) { [weak self] in
                let actual = try $0.setOutputEnable(output: Self.wire(output), enabled: enabled)
                guard actual != enabled else { return }
                DispatchQueue.main.async { self?.outputEnabled[output] = actual }
            }
            return
        }
        var val: UInt8 = enabled ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_ENABLE, value: UInt16(output), index: 2, data: data)
    }

    func fetchOutputEnable(output: Int) {
        // -- WP SW9 -- Through a core, its mirror: its setter read it back.
        if let core = typedCore {
            let read = core.call { core -> Bool? in
                let enabled = core.getState().outputEnabled
                return enabled.indices.contains(output) ? enabled[output] : nil
            }
            guard case .success(let val?) = read else { return }
            DispatchQueue.main.async { if self.outputEnabled[output] != val { self.outputEnabled[output] = val } }
            return
        }
        if let d = transport.getControlRequest(request: REQ_GET_OUTPUT_ENABLE, value: UInt16(output), index: 2, length: 1) {
            let val = d[0] != 0
            DispatchQueue.main.async { self.outputEnabled[output] = val }
        }
    }

    // MARK: - Per-Output Gain

    func setOutputGain(output: Int, db: Float) {
        var val = (db * 10).rounded() / 10
        if val == -0.0 { val = 0.0 }
        outputGainDB[output] = val
        // A drag's preview hands over to the committed value in the same pass,
        // so the curve does not flick back to the pre-drag gain.
        if outputGainPreview.gains[output] != nil { outputGainPreview.gains[output] = nil }
        if let core = typedCore {   // -- WP SW9 --
            core.commit(lane: "out.gain[\(output)]") { [val] in   // -- fixup sw9 F2 --
                try $0.setOutputGain(output: Self.wire(output), db: val)
            }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_GAIN, value: UInt16(output), index: 2, data: data)
    }

    // MARK: - Per-Output Mute

    func setOutputMute(output: Int, muted: Bool) {
        outputMuted[output] = muted
        if let core = typedCore {   // -- WP SW9 --
            core.write(lane: nil) { try $0.setOutputMute(output: Self.wire(output), muted: muted) }
            return
        }
        var val: UInt8 = muted ? 1 : 0
        let data = Data(bytes: &val, count: 1)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_MUTE, value: UInt16(output), index: 2, data: data)
    }

    // MARK: - Per-Output Delay

    func setOutputDelay(output: Int, ms: Float) {
        var val = Self.quantizeDelayMS(ms)   // -- WP SWF5 --
        // -- WP SWF8 -- Past the device's delay line (or not a number):
        // nothing is sent, the field shows the delay it had, and the status
        // row says why.
        guard val <= maxOutputDelayMS else { refuseOutputDelay(output: output, ms: val); return }
        outputDelayMS[output] = val
        if let core = typedCore {   // -- WP SW9 --
            core.commit(lane: "out.delay[\(output)]") { [val] in   // -- fixup sw9 F2 --
                try $0.setOutputDelay(output: Self.wire(output), ms: val)
            }
            return
        }
        let data = Data(bytes: &val, count: 4)
        transport.sendControlRequest(request: REQ_SET_OUTPUT_DELAY, value: UInt16(output), index: 2, data: data)
    }

    // -- WP SWF8 -- c75-N3
    /// The longest output delay this device's line holds, in ms: the core's
    /// `maxDelayMs` while the view model follows a core, else the same rule
    /// (dspi_proto Platform::max_delay_ms: 42 ms on RP2350, 21 ms on RP2040
    /// and anything else) from the platform.
    var maxOutputDelayMS: Float {
        if coreMirror.attachedSource != nil, let core = coreMaxDelayMS { return core }
        return Self.delayLineMS(platform: platformName)
    }

    static func delayLineMS(platform: String) -> Float { platform == "RP2350" ? 42 : 21 }

    /// A delay past the line: re-publish the delay kept, so the fields
    /// show it again, and post the refusal, in the core's words through the
    /// core (its typed setter refuses before anything is sent).
    private func refuseOutputDelay(output: Int, ms: Float) {
        outputDelayMS[output] = outputDelayMS[output]
        let max = maxOutputDelayMS
        let text = "out.delay: \(ValueField.delayText(ms)) ms is out of range; this device"
            + " (\(platformName.isEmpty ? "RP2040" : platformName)) holds 0 to \(ValueField.delayText(max)) ms"
        guard let source = coreMirror.attachedSource, let wire = UInt8(exactly: output) else {
            DeviceLock.notice.post(text)
            return
        }
        source.outputDelayRefusal(output: wire, ms: ms) { DeviceLock.notice.post($0 ?? text) }
    }

    // MARK: - Core 1 Mode

    func fetchCore1Mode() {
        if let core = typedCore {   // -- WP SW9 --
            guard case .success(let mode) = core.call({ try $0.fetchCore1Mode() }) else { return }
            DispatchQueue.main.async { self.core1Mode = Int(mode) }
            return
        }
        if let d = transport.getControlRequest(request: REQ_GET_CORE1_MODE, value: 0, index: 0, length: 1) {
            let val = Int(d[0])
            DispatchQueue.main.async { self.core1Mode = val }
        }
    }

    /// Disable all EQ worker outputs, then enable PDM, confirm via read-back.
    func switchToPDM() {
        for i in eqWorkerRange {
            setOutputEnable(output: i, enabled: false)
        }
        setOutputEnable(output: pdmOutputIndex, enabled: true)
        // Read back to confirm
        for i in eqWorkerRange {
            fetchOutputEnable(output: i)
        }
        fetchOutputEnable(output: pdmOutputIndex)
        fetchCore1Mode()
    }

    /// Disable PDM, then enable the requested output, confirm via read-back.
    func switchFromPDM(enabling output: Int) {
        setOutputEnable(output: pdmOutputIndex, enabled: false)
        setOutputEnable(output: output, enabled: true)
        // Read back to confirm
        fetchOutputEnable(output: pdmOutputIndex)
        fetchOutputEnable(output: output)
        fetchCore1Mode()
    }

    // MARK: - Pin Configuration

    // -- WP SW11 -- Through a core, the pins, I2S, ADAT, inputs, UART/I2C,
    // LG Sound Sync, DAC mute and names call its typed API: the core checks
    // and journals each change (Undo steps back what its registry calls
    // undoable: channel names, LG Sound Sync, the master clock; the pins,
    // buses, inputs and the flash-written names are recorded and stepped
    // over, as in Qt), and the settings are read from its mirror, which
    // CoreMirror shows.  The setters that return a PIN_CONFIG_* status
    // block for the core's answer (`writeAndWait`).  Without a core (USB without
    // -CoreTransport, MIDI, the Link client) they stay raw SETs and GETs.

    /// -- WP SW11 -- A typed setter's outcome as the PIN_CONFIG_* status the
    /// raw request returns: the device's refusal carries it, any other
    /// failure is 0xFF.
    static func pinStatus(_ result: Result<Void, DspiError>) -> UInt8 {
        switch result {
        case .success: return PIN_CONFIG_SUCCESS
        case .failure(.Refused(let status)): return status
        case .failure: return 0xFF
        }
    }

    /// -- WP SW11 -- Through a core, `body` as a typed call with a status
    /// (blocking, then the state is published); nil without one, for the raw
    /// request.
    private func typedPinStatus(_ body: (DspiCore) throws -> Void) -> UInt8? {
        guard let core = typedCore else { return nil }
        return Self.pinStatus(core.writeAndWait(body))
    }

    /// -- WP SW11 -- Whether the core finds `feature` on the device (no I/O).
    private func coreFeature(_ core: CoreStateSource, _ feature: String) -> Bool {
        (try? core.call { $0.featurePresent(name: feature) }.get()) ?? false
    }

    func fetchOutputPin(output: Int) {
        if let core = typedCore {   // -- WP SW11 -- its re-read, into the mirror shown
            _ = core.readAndPublish { _ = try $0.fetchOutputPin(slot: UInt8(output)) }
            return
        }
        if let d = transport.getControlRequest(request: REQ_GET_OUTPUT_PIN, value: UInt16(output), index: 2, length: 1) {
            let pin = d[0]
            DispatchQueue.main.async {
                self.outputPins[output] = pin
            }
        }
    }

    /// Sets the GPIO pin for a physical output. Returns the firmware status code.
    /// Single IN transfer: wValue = (new_pin << 8) | output_index
    @discardableResult
    func setOutputPin(output: Int, pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setOutputPin(slot: UInt8(output), pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.outputPins[output] = pin } }
            return status
        }
        let wValue = (UInt16(pin) << 8) | UInt16(output)
        if let d = transport.getControlRequest(request: REQ_SET_OUTPUT_PIN, value: wValue, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.outputPins[output] = pin
                }
            }
            return status
        }
        return 0xFF
    }

    /// Sets an output's GPIO pin, transparently cycling the PDM output off and
    /// back on when the firmware refuses because it is active.  The one shared
    /// path for pin assignment, used by Settings and the Getting Started
    /// wizard, so the retry rule cannot drift between the two.
    @discardableResult
    func assignOutputPin(output: Int, pin: UInt8) -> UInt8 {
        var status = setOutputPin(output: output, pin: pin)
        if status == PIN_CONFIG_OUTPUT_ACTIVE && output == pdmPinIndex {
            setOutputEnable(output: pdmOutputIndex, enabled: false)
            status = setOutputPin(output: output, pin: pin)
            setOutputEnable(output: pdmOutputIndex, enabled: true)
        }
        return status
    }

    // MARK: - I2S Configuration

    /// Switch an output slot between S/PDIF (0) and I2S (1). Returns firmware status code.
    @discardableResult
    func setOutputSlotType(slot: Int, type: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setOutputType(slot: UInt8(slot), kind: type) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.outputSlotTypes[slot] = type } }
            return status
        }
        let wValue = (UInt16(type) << 8) | UInt16(slot)
        if let d = transport.getControlRequest(request: REQ_SET_OUTPUT_TYPE, value: wValue, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.outputSlotTypes[slot] = type }
            }
            return status
        }
        return 0xFF
    }

    func fetchOutputSlotType(slot: Int) {
        // -- WP SW11 -- Through a core, its mirror holds the types; a re-read
        // of them is `refreshOutputSlotTypes`.
        if typedCore != nil { return }
        if let d = transport.getControlRequest(request: REQ_GET_OUTPUT_TYPE, value: UInt16(slot), index: 2, length: 1) {
            let type = d[0]
            DispatchQueue.main.async { self.outputSlotTypes[slot] = type }
        }
    }

    /// Re-read every slot's bus (STM32 firmware coerces the dependent slots
    /// when a parent goes S/PDIF).  Blocking.  -- WP SW11 -- Through a core,
    /// its full re-read: it has no per-slot read, its mirror is published.
    func refreshOutputSlotTypes() {
        if let core = typedCore {
            _ = core.readAndPublish { try $0.refreshParams() }
            return
        }
        for slot in 0..<numOutputSlots { fetchOutputSlotType(slot: slot) }
    }

    /// Set a BCK GPIO pin (LRCLK = BCK + 1).  `role` selects the master/unified
    /// pair (0, the default and legacy behavior) or the slave pair (1, SPLIT
    /// clock-pin mode only; storable any time while dormant).  wValue packs the
    /// role in its high byte.  The master pair requires all slots to be S/PDIF
    /// when it is the one clocking outputs (clock_pins_spec.md §3).
    @discardableResult
    func setI2SBckPin(_ pin: UInt8, role: UInt8 = I2S_BCK_ROLE_MASTER) -> UInt8 {
        if let status = typedPinStatus({ try $0.setI2sBckPin(role: role, pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if role == I2S_BCK_ROLE_SLAVE { self.i2sBckPinSlave = pin } else { self.i2sBckPin = pin }
                }
            }
            return status
        }
        let wValue = (UInt16(role) << 8) | UInt16(pin)
        if let d = transport.getControlRequest(request: REQ_SET_I2S_BCK_PIN, value: wValue, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if role == I2S_BCK_ROLE_SLAVE { self.i2sBckPinSlave = pin }
                    else { self.i2sBckPin = pin }
                }
            }
            return status
        }
        return 0xFF
    }

    func fetchI2SBckPin(role: UInt8 = I2S_BCK_ROLE_MASTER) {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds both pairs
        if let d = transport.getControlRequest(request: REQ_GET_I2S_BCK_PIN, value: UInt16(role), index: 2, length: 1) {
            let pin = d[0]
            DispatchQueue.main.async {
                if role == I2S_BCK_ROLE_SLAVE { self.i2sBckPinSlave = pin }
                else { self.i2sBckPin = pin }
            }
        }
    }

    /// Enable or disable the master clock output.
    @discardableResult
    func setMckEnable(_ enabled: Bool) -> UInt8 {
        if let status = typedPinStatus({ try $0.setMckEnabled(enabled: enabled) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.mckEnabled = enabled } }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_MCK_ENABLE, value: enabled ? 1 : 0, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.mckEnabled = enabled }
            }
            return status
        }
        return 0xFF
    }

    func fetchMckEnable() {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds it
        if let d = transport.getControlRequest(request: REQ_GET_MCK_ENABLE, value: 0, index: 2, length: 1) {
            let val = d[0] != 0
            DispatchQueue.main.async { self.mckEnabled = val }
        }
    }

    /// Set MCK GPIO pin. MCK must be disabled first.
    @discardableResult
    func setMckPin(_ pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setMckPin(pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.mckPin = pin } }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_MCK_PIN, value: UInt16(pin), index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.mckPin = pin }
            }
            return status
        }
        return 0xFF
    }

    func fetchMckPin() {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds it
        if let d = transport.getControlRequest(request: REQ_GET_MCK_PIN, value: 0, index: 2, length: 1) {
            DispatchQueue.main.async { self.mckPin = d[0] }
        }
    }

    /// Set MCK frequency multiplier (128 or 256).
    /// Wire encoding (V5+): wValue 0 = 128x, 1 = 256x.
    @discardableResult
    func setMckMultiplier(_ multiplier: Int) -> UInt8 {
        let wireValue: UInt16 = (multiplier == 256) ? 1 : 0
        if let status = typedPinStatus({ try $0.setMckMultiplier(mult: UInt8(wireValue)) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.mckMultiplier = multiplier } }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_MCK_MULTIPLIER, value: wireValue, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.mckMultiplier = multiplier }
            }
            return status
        }
        return 0xFF
    }

    func fetchMckMultiplier() {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds it
        if let d = transport.getControlRequest(request: REQ_GET_MCK_MULTIPLIER, value: 0, index: 2, length: 1) {
            let raw = d[0]
            let value = raw == 1 ? 256 : 128  // V5 encoding: 0 = 128x, 1 = 256x
            DispatchQueue.main.async { self.mckMultiplier = value }
        }
    }

    // MARK: - Stream states and names through the core

    /// -- WP SW11 -- What a core's notification said beyond the mirror,
    /// which CoreMirror has applied already.  A stream state (CG25's
    /// NOTIFY_STREAMS: ADAT out, the I2S slave and ADAT input locks) is read
    /// again in full; another client's source or device name is read from the
    /// core's copy (no traffic).  Without a core the interrupt monitor's
    /// hooks do this (DSPViewModel).  Main thread.
    func followCoreEvents(_ flags: UInt32) {
        guard transport.coreStateSource != nil else { return }
        if flags & DspiConstants.NOTIFY_STREAMS != 0 {
            let (adat, i2s, adatIn) = (adatSupported, i2sClockModeSupported, adatInputSupported)
            DispatchQueue.global(qos: .userInitiated).async {
                if adat { self.fetchAdatStatus() }
                if i2s { self.fetchI2SSlaveStatus() }
                if adatIn { self.fetchAdatInputStatus() }
            }
        }
        if flags & DspiConstants.NOTIFY_NAMES != 0 {
            DispatchQueue.global(qos: .userInitiated).async {
                self.fetchInputSourceNames()
                self.fetchDeviceName()
            }
        }
    }

    // MARK: - ADAT Bulk Output Commands
    //
    // RP2350-only ADAT lightpipe of all 8 main output channels on one GPIO.
    // See adat_output_spec.md.  Both SETs are IN-direction transfers carrying
    // the value in wValue and returning a PIN_CONFIG_* status byte (same shape
    // as REQ_SET_SPDIF_RX_PIN); on RP2040 they return INVALID_OUTPUT.

    /// Probe + refresh live ADAT state via REQ_GET_ADAT_STATUS.  A STALL (nil)
    /// means the firmware predates ADAT; combined with the RP2350 gate this sets
    /// `adatSupported`.  Called from `fetchAll` after the bulk fetch.
    func fetchAdatStatus() {
        // -- WP SW11 -- Through a core, the feature is the core's (and RP2350
        // only, as below), as Qt gates it; a failed status read keeps the
        // last status.
        if let core = typedCore {
            let generation = transport.generation
            let supported = coreFeature(core, "adat_output")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                let shown = supported && self.platformName == "RP2350"
                if self.adatSupported != shown { self.adatSupported = shown }
            }
            guard supported, case .success(let s) = core.call({ try $0.fetchAdatOutStatus() }) else { return }
            var status = AdatStatus()
            status.enabled = s.enabled
            status.active = s.active
            status.pin = s.pin
            status.rateOk = s.rateOk
            status.resyncCount = s.resyncCount
            status.slipCount = s.slipCount
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.adatStatus != status { self.adatStatus = status }
            }
            return
        }
        guard let data = transport.getControlRequest(request: REQ_GET_ADAT_STATUS, value: 0, index: 2, length: 8),
              let status = AdatStatus.fromData(data) else {
            DispatchQueue.main.async { self.adatSupported = false }
            return
        }
        DispatchQueue.main.async {
            // The engine is compiled out on RP2040 (status all-zero); gate the UI
            // on platform so a zeroed RP2040 response never shows the section.
            self.adatSupported = (self.platformName == "RP2350")
            self.adatStatus = status
            self.adatEnabled = status.enabled
            if status.pin != 0 { self.adatPin = status.pin }
        }
    }

    /// Alias kept parallel to the other `fetch…Config` probes used by `fetchAll`.
    func fetchAdatConfig() { fetchAdatStatus() }

    /// Enable or disable the ADAT bulk output.  Enabling validates the
    /// configured pin first (INVALID_PIN / PIN_IN_USE on conflict); disabling
    /// always succeeds.  Returns the firmware status byte (0xFF on transfer
    /// failure).
    @discardableResult
    func setAdatEnable(_ enabled: Bool) -> UInt8 {
        if let status = typedPinStatus({ try $0.setAdatOutEnabled(enabled: enabled) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatEnabled = enabled
                    self.adatStatus.enabled = enabled
                }
            }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_ADAT_ENABLE, value: enabled ? 1 : 0, index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatEnabled = enabled
                    self.adatStatus.enabled = enabled
                }
            }
            return status
        }
        return 0xFF
    }

    /// Set the ADAT data GPIO.  `wValue = 0` resets to the platform default
    /// (ADAT_PIN_DEFAULT).  May be issued while enabled; the firmware re-routes
    /// under a muted restart.  Returns the firmware status byte.
    @discardableResult
    func setAdatPin(_ pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setAdatOutPin(pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                let applied = pin == 0 ? ADAT_PIN_DEFAULT : pin
                DispatchQueue.main.async {
                    self.adatPin = applied
                    self.adatStatus.pin = applied
                }
            }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_ADAT_PIN, value: UInt16(pin), index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                let applied = pin == 0 ? ADAT_PIN_DEFAULT : pin
                DispatchQueue.main.async {
                    self.adatPin = applied
                    self.adatStatus.pin = applied
                }
            }
            return status
        }
        return 0xFF
    }

    // MARK: - ADAT Input Commands
    //
    // RP2350-only selectable 8-channel ADAT input (INPUT_SOURCE_ADAT = 3).  See
    // adat_input_spec.md.  Both SETs are IN-direction transfers carrying the value
    // in wValue and returning a PIN_CONFIG_* status byte; on RP2040 0x68/0x6A
    // return INVALID_OUTPUT and 0x6E returns zeros (the config GETs round-trip).

    /// Probe + refresh live ADAT input state via REQ_GET_ADAT_INPUT_STATUS (0x6E).
    /// A STALL (nil) means the firmware predates ADAT input; combined with the
    /// RP2350 gate this sets `adatInputSupported`.  Called from `fetchAll`.
    func fetchAdatInputConfig() {
        if let core = typedCore {   // -- WP SW11 -- the feature is the core's; the settings are its mirror's
            let generation = transport.generation
            guard coreFeature(core, "adat_input") else {
                DispatchQueue.main.async { if self.transport.generation == generation { self.adatInputSupported = false } }
                return
            }
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                self.adatInputSupported = (self.platformName == "RP2350")
            }
            fetchAdatInputStatus()
            return
        }
        guard let data = transport.getControlRequest(request: REQ_GET_ADAT_INPUT_STATUS, value: 0, index: 2, length: 20),
              let status = AdatInputStatus.fromData(data) else {
            DispatchQueue.main.async { self.adatInputSupported = false }
            return
        }
        DispatchQueue.main.async {
            // The engine is compiled out on RP2040 (status all-zero); gate the UI
            // on platform so a zeroed RP2040 response never shows the section.
            self.adatInputSupported = (self.platformName == "RP2350")
            self.adatInputStatus = status
            self.adatInputEnabled = status.enabled
            self.adatInputClockMode = status.clockMode
            self.adatInputPin = status.pin
        }
    }

    /// Refresh only the live 20-byte AdatInputStatusPacket (lock state, counts,
    /// detected/measured rate) for the settings lock indicator.  Also re-syncs the
    /// configured enable / pin / clock mode the packet carries.
    func fetchAdatInputStatus() {
        // -- WP SW11 -- Through a core, its lock read (input 1); the settings
        // the raw packet carries come from its mirror (CoreMirror).
        if let core = typedCore {
            let generation = transport.generation
            guard case .success(let lock) = core.call({ try $0.fetchInputLock(which: 1) }) else { return }
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                var status = self.adatInputStatus
                status.state = lock.state
                status.clockMode = lock.clockMode
                status.rateOk = lock.rateOk
                status.lockCount = lock.lockCount
                status.lossCount = lock.lossCount
                status.detectedRate = lock.detectedRate
                status.measuredHz = lock.measuredHz
                status.enabled = self.adatInputEnabled
                status.pin = self.adatInputPin
                if self.adatInputStatus != status { self.adatInputStatus = status }
            }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_ADAT_INPUT_STATUS, value: 0, index: 2, length: 20),
              let status = AdatInputStatus.fromData(d) else { return }
        DispatchQueue.main.async {
            self.adatInputStatus = status
            self.adatInputClockMode = status.clockMode
            self.adatInputEnabled = status.enabled
            self.adatInputPin = status.pin
        }
    }

    /// Enable or disable the ADAT input.  Enabling validates the configured pin
    /// first (INVALID_PIN when unset/invalid, PIN_IN_USE on conflict); disabling
    /// is refused (PIN_IN_USE) while ADAT is the active source.  Returns the
    /// firmware PIN_CONFIG_* status byte (0xFF on transfer failure).
    @discardableResult
    func setAdatInputEnable(_ enabled: Bool) -> UInt8 {
        if let status = typedPinStatus({ try $0.setAdatInputEnabled(enabled: enabled) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputEnabled = enabled
                    self.adatInputStatus.enabled = enabled
                }
            }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_ADAT_INPUT_ENABLE, value: enabled ? 1 : 0, index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputEnabled = enabled
                    self.adatInputStatus.enabled = enabled
                }
            }
            return status
        }
        return 0xFF
    }

    /// Set the ADAT input RX GPIO (or 0xFF to clear, allowed only while disabled).
    /// The pin MAY equal the ADAT output pin (loopback self-test).  May be issued
    /// while ADAT is the live source; the firmware re-routes under a muted restart.
    /// Returns the firmware PIN_CONFIG_* status byte.
    @discardableResult
    func setAdatInputPin(_ pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setAdatInputPin(pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputPin = pin
                    self.adatInputStatus.pin = pin
                }
            }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_ADAT_INPUT_PIN, value: UInt16(pin), index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputPin = pin
                    self.adatInputStatus.pin = pin
                }
            }
            return status
        }
        return 0xFF
    }

    /// Select the ADAT input clock mode (0=master, 1=slave) via 0x6C.  Deferred:
    /// the firmware applies it in its main loop (instantly when ADAT is not the
    /// active source, otherwise under a muted receiver restart).  Returns the
    /// PIN_CONFIG_* status byte; the live mode is confirmed via the
    /// adat_clock_mode_p1 PARAM_CHANGED (or the next NOTIFY 0x0B).  Part of the
    /// output-config block, so callers must mark it dirty (beginOutputEdit).
    @discardableResult
    func setAdatInputClockMode(_ mode: UInt8) -> UInt8 {
        let clamped: UInt8 = mode == ADAT_INPUT_CLOCK_MODE_SLAVE ? ADAT_INPUT_CLOCK_MODE_SLAVE : ADAT_INPUT_CLOCK_MODE_MASTER
        if let status = typedPinStatus({ try $0.setAdatInputClockMode(mode: clamped) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputClockMode = clamped
                    self.adatInputStatus.clockMode = clamped
                }
            }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_ADAT_INPUT_CLOCK_MODE, value: UInt16(clamped), index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    self.adatInputClockMode = clamped
                    self.adatInputStatus.clockMode = clamped
                }
            }
            return status
        }
        return 0xFF
    }

    // MARK: - Input Source Commands

    /// Probes GET_INPUT_SOURCE. If the device STALLs (nil), the feature is unsupported.
    func fetchInputSource() {
        // -- WP SW11 -- Through a core, the source is its mirror's and the
        // feature its feature list's; then the inputs' reads, through it too.
        if let core = typedCore {
            let generation = transport.generation
            let supported = coreFeature(core, "input_source")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.inputSourceSupported != supported { self.inputSourceSupported = supported }
            }
            if supported {
                fetchSpdifInputConfig()
                fetchI2SInputConfig()
            }
            fetchInputSourceNames()
            fetchDeviceName()
            return
        }
        let data = transport.getControlRequest(request: REQ_GET_INPUT_SOURCE, value: 0, index: 2, length: 1)
        DispatchQueue.main.async {
            if let data = data, data.count >= 1 {
                self.inputSourceSupported = true
                self.inputSource = Int(data[0])
            } else {
                self.inputSourceSupported = false
                self.inputSource = 0
            }
        }
        if data != nil {
            fetchSpdifInputConfig()
            fetchI2SInputConfig()
        }
        // Unconditional, so a device switch never keeps the last device's names.
        fetchInputSourceNames()
        fetchDeviceName()
    }

    /// Read every source name.  Older firmware answers source 0 with its 4-byte
    /// legacy gain, so only that probe decides support; a later failed read
    /// just leaves that source on its built-in label.
    func fetchInputSourceNames() {
        // -- WP SW11 -- Through a core, the names it read on connect and
        // keeps current (no traffic).  `Unsupported` on source 0: no names.
        if let core = typedCore {
            let generation = transport.generation
            var names = Array(repeating: "", count: INPUT_SOURCE_NAME_SLOTS)
            var supported = false
            for source in 0...INPUT_SOURCE_MAX {
                let r = core.call { try $0.cachedInputSourceName(source: UInt8(source)) }
                if source == 0 {
                    guard case .success = r else { break }
                    supported = true
                }
                if case .success(let name) = r { names[source] = name }
            }
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                self.inputSourceNamesSupported = supported
                let shown = supported ? names : Array(repeating: "", count: INPUT_SOURCE_NAME_SLOTS)
                if self.inputSourceNames != shown { self.inputSourceNames = shown }
            }
            return
        }
        var names = Array(repeating: "", count: INPUT_SOURCE_NAME_SLOTS)
        var supported = false
        for source in 0...INPUT_SOURCE_MAX {
            let d = transport.getControlRequest(request: REQ_GET_INPUT_SOURCE_NAME, value: UInt16(source),
                                                index: 2, length: UInt16(INPUT_SOURCE_NAME_LEN))
            let ok = d?.count == INPUT_SOURCE_NAME_LEN
            if source == 0 {
                guard ok else { break }
                supported = true
            }
            if ok, let d { names[source] = DSPViewModel.decodeInputSourceName(d) }
        }
        DispatchQueue.main.async {
            self.inputSourceNamesSupported = supported
            self.inputSourceNames = supported ? names : Array(repeating: "", count: INPUT_SOURCE_NAME_SLOTS)
        }
    }

    /// Store a source name; an empty name restores the built-in label.  The
    /// device writes flash on its main loop and echoes the stored name back as
    /// NOTIFY_EVT_INPUT_SOURCE_NAME.
    func setInputSourceName(source: Int, name: String) {
        guard inputSourceNamesSupported, (0...INPUT_SOURCE_MAX).contains(source) else { return }
        let ascii = DSPViewModel.sanitizeInputSourceName(name)
        DispatchQueue.main.async {
            self.inputSourceNames[source] = ascii
        }
        if typedWrite({ try $0.setInputSourceName(source: UInt8(source), name: ascii) }) { return }   // -- WP SW11 --
        var payload = Data(count: INPUT_SOURCE_NAME_LEN)
        payload.replaceSubrange(0..<ascii.utf8.count, with: Data(ascii.utf8))
        transport.sendControlRequest(request: REQ_SET_INPUT_SOURCE_NAME, value: UInt16(source), index: 2, data: payload)
    }

    /// Read the board name.  The probe is this GET alone: a STALL or a reply
    /// shorter than 32 bytes means the firmware has no board name.
    func fetchDeviceName() {
        // A late reply from the previous device must not land on the new one.
        let generation = transport.generation
        // -- WP SW11 -- Through a core, the name it keeps (no traffic);
        // `Unsupported`: the firmware has none.
        if let core = typedCore {
            let r = core.call { try $0.cachedDeviceName() }
            let name = (try? r.get()) ?? ""
            let supported: Bool
            if case .success = r { supported = true } else { supported = false }
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                self.deviceNameSupported = supported
                if self.deviceName != name { self.deviceName = name }
            }
            return
        }
        let d = transport.getControlRequest(request: REQ_GET_DEVICE_NAME, value: 0,
                                            index: 2, length: UInt16(DEVICE_NAME_LEN))
        let supported = d?.count == DEVICE_NAME_LEN
        let name = supported ? DSPViewModel.decodeInputSourceName(d!) : ""
        DispatchQueue.main.async {
            guard self.transport.generation == generation else { return }
            self.deviceNameSupported = supported
            self.deviceName = name
        }
    }

    /// Store the board name; empty clears it.  The device writes flash on its
    /// main loop and echoes the stored name as NOTIFY_EVT_DEVICE_NAME.
    func setDeviceName(_ name: String) {
        guard deviceNameSupported else { return }
        let ascii = DSPViewModel.sanitizeInputSourceName(name)
        DispatchQueue.main.async {
            self.deviceName = ascii
        }
        if typedWrite({ try $0.setDeviceName(name: ascii) }) { return }   // -- WP SW11 --
        var payload = Data(count: DEVICE_NAME_LEN)
        payload.replaceSubrange(0..<ascii.utf8.count, with: Data(ascii.utf8))
        transport.sendControlRequest(request: REQ_SET_DEVICE_NAME, value: 0, index: 2, data: payload)
    }

    /// Reads the whole S/PDIF input inventory in one transfer (REQ_GET_SPDIF_INPUT_CONFIG,
    /// firmware v1.1.5+): input count, enable mask, and each input's GPIO pin.  On
    /// older firmware the request STALLs; we fall back to the single-input pin read
    /// (REQ_GET_SPDIF_RX_PIN index 0) and mark multi-input as unsupported.
    func fetchSpdifInputConfig() {
        // -- WP SW11 -- Through a core, the pins and the enable mask are its
        // mirror's; more than one input is its feature, then four, as in Qt.
        if let core = typedCore {
            let generation = transport.generation
            let multi = coreFeature(core, "spdif_multi_input")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                self.multiSpdifSupported = multi
                self.spdifInputCount = multi ? SPDIF_RX_NUM_INPUTS : 1
            }
            return
        }
        // The response is 2 + count bytes.  Ask for the largest inventory we know
        // about; a three-input firmware answers short, which is why the pin list
        // is sized from the returned count rather than a fixed length.
        let maxLen = UInt16(2 + SPDIF_RX_NUM_INPUTS)
        guard let d = transport.getControlRequest(request: REQ_GET_SPDIF_INPUT_CONFIG, value: 0, index: 2, length: maxLen),
              d.count >= 3 else {
            DispatchQueue.main.async {
                self.multiSpdifSupported = false
                self.spdifInputCount = 1
            }
            fetchSpdifRxPin()
            return
        }
        let count = min(Int(d[0]), min(d.count - 2, SPDIF_RX_NUM_INPUTS))
        let mask = d[1]
        let pin0 = d[2]
        // Keep the ext arrays at full length whatever the device reports, so a
        // three-input device still has a well-formed slot for a future input 4;
        // indices past `count` stay at their defaults and are never shown.
        var pinsExt = Array(SPDIF_RX_PIN_DEFAULTS.dropFirst())
        var enabled = Array(repeating: false, count: SPDIF_RX_NUM_INPUTS - 1)
        for idx in 1..<max(count, 1) {
            pinsExt[idx - 1] = d[2 + idx]
            enabled[idx - 1] = (mask & (1 << UInt8(idx))) != 0
        }
        DispatchQueue.main.async {
            self.multiSpdifSupported = true
            self.spdifInputCount = max(count, 1)
            self.spdifRxPin = pin0
            self.spdifRxPinsExt = pinsExt
            self.spdifExtEnabled = enabled
        }
    }

    func setInputSource(_ source: Int) {
        let byte = Data([UInt8(source)])

        DispatchQueue.main.async {
            self.inputSource = source
        }
        if typedWrite({ try $0.setInputSource(source: UInt8(source)) }) { return }   // -- WP SW11 --

        transport.sendControlRequest(request: REQ_SET_INPUT_SOURCE, value: 0, index: 2, data: byte)

        // No follow-up fetch here — the firmware emits an
        // `input_config.input_source` notification at the end of its
        // deferred switch handler (main.c:1662).  applyNotifiedParamChange
        // catches that and triggers fetchUserVolume() at the right
        // moment instead of guessing a sleep duration.
    }

    /// Reads a single S/PDIF input's GPIO pin (default index 0 = input 1).
    func fetchSpdifRxPin(index: Int = 0) {
        if let core = typedCore {   // -- WP SW11 -- its re-read, into the mirror shown
            _ = core.readAndPublish { _ = try $0.fetchSpdifRxPin(index: UInt8(index)) }
            return
        }
        if let data = transport.getControlRequest(request: REQ_GET_SPDIF_RX_PIN, value: UInt16(index), index: 2, length: 1),
           data.count >= 1 {
            let pin = data[0]
            DispatchQueue.main.async {
                if index == 0 { self.spdifRxPin = pin }
                else if self.spdifRxPinsExt.indices.contains(index - 1) { self.spdifRxPinsExt[index - 1] = pin }
            }
        }
    }

    /// Set the GPIO pin used for a S/PDIF input (index 0..3, default 0 = input 1).
    /// Single IN transfer: wValue = (index << 8) | pin.  Returns the firmware
    /// PIN_CONFIG_* status code.  The pin is hot-swappable while the input is
    /// active (no OUTPUT_ACTIVE rejection); a disabled optional input stores the
    /// pin as a preference validated at enable time.
    @discardableResult
    func setSpdifRxPin(index: Int = 0, _ pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setSpdifRxPin(index: UInt8(index), pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if index == 0 { self.spdifRxPin = pin }
                    else if self.spdifRxPinsExt.indices.contains(index - 1) { self.spdifRxPinsExt[index - 1] = pin }
                }
            }
            return status
        }
        let wValue = UInt16((index << 8) | Int(pin))
        if let d = transport.getControlRequest(request: REQ_SET_SPDIF_RX_PIN, value: wValue, index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if index == 0 { self.spdifRxPin = pin }
                    else if self.spdifRxPinsExt.indices.contains(index - 1) { self.spdifRxPinsExt[index - 1] = pin }
                }
            }
            return status
        }
        return 0xFF
    }

    /// Enable or disable an optional S/PDIF input (index 1..3 = SPDIF 2/3/4).
    /// Input 1 (index 0) is always enabled.  IN transfer: wValue = (index<<8)|enable.
    /// Returns the firmware PIN_CONFIG_* status.  Enabling validates the configured
    /// pin; disabling is refused (PIN_IN_USE) while the input is the active source.
    @discardableResult
    func setSpdifInputEnable(index: Int, _ enable: Bool) -> UInt8 {
        if let status = typedPinStatus({ try $0.setSpdifInputEnabled(index: UInt8(index), enabled: enable) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS, index >= 1, self.spdifExtEnabled.indices.contains(index - 1) {
                let i = index - 1
                DispatchQueue.main.async { self.spdifExtEnabled[i] = enable }
            }
            return status
        }
        let wValue = UInt16((index << 8) | (enable ? 1 : 0))
        if let d = transport.getControlRequest(request: REQ_SET_SPDIF_INPUT_ENABLE, value: wValue, index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS, index >= 1, self.spdifExtEnabled.indices.contains(index - 1) {
                let i = index - 1
                DispatchQueue.main.async { self.spdifExtEnabled[i] = enable }
            }
            return status
        }
        return 0xFF
    }

    // MARK: - I2S Input Commands

    /// Probes I2S input support (firmware V12+).  REQ_GET_I2S_RX_PIN STALLs on
    /// older firmware; on success we read every stereo pair's data pin, the
    /// active channel count, the shared BCK pin, and the selected input rate.
    func fetchI2SInputConfig() {
        // -- WP SW11 -- Through a core, the pins, channels and rate are its
        // mirror's; the features are its feature list's.
        if let core = typedCore {
            let generation = transport.generation
            let supported = coreFeature(core, "i2s_input_channels")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.i2sInputSupported != supported { self.i2sInputSupported = supported }
            }
            guard supported else { return }
            fetchI2SClockMode()
            fetchI2SClockPinMode()
            return
        }
        guard let data = transport.getControlRequest(request: REQ_GET_I2S_RX_PIN, value: 0, index: 2, length: 1),
              data.count >= 1 else {
            DispatchQueue.main.async { self.i2sInputSupported = false }
            return
        }
        DispatchQueue.main.async {
            self.i2sInputSupported = true
            self.i2sRxPins[0] = data[0]
        }
        // Extra stereo pairs (RP2350).  Older/stereo firmware returns 0 for
        // pairs it doesn't have; keep the default in that case.
        for pair in 1..<I2S_RX_MAX_PAIRS_RP2350 {
            if let d = transport.getControlRequest(request: REQ_GET_I2S_RX_PIN, value: UInt16(pair), index: 2, length: 1),
               d.count >= 1, d[0] != 0 {
                let p = d[0]
                DispatchQueue.main.async { self.i2sRxPins[pair] = p }
            }
        }
        fetchI2SInputChannels()
        fetchInputRate()
        fetchI2SClockMode()
        fetchI2SClockPinMode()
    }

    /// Reads the active I2S input channel count (2/4/6/8).  STALLs on firmware
    /// that predates multichannel I2S; leaves the default (2) in that case.
    func fetchI2SInputChannels() {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds it
        guard let d = transport.getControlRequest(request: REQ_GET_I2S_INPUT_CHANNELS, value: 0, index: 2, length: 1),
              d.count >= 1, d[0] != 0 else { return }
        let count = Int(d[0])
        DispatchQueue.main.async { self.i2sInputChannels = count }
    }

    /// Selects the active I2S input channel count (2/4/6/8 → 1..4 stereo pairs).
    /// IN transfer returning a PIN_CONFIG_* status: a raise validates each newly
    /// activated pair's data pin and is rejected (without changing state) on a
    /// clash; RP2040 rejects anything but 2.  Part of the output-config block,
    /// so callers should mark it dirty (beginOutputEdit).  On success the device
    /// pushes NOTIFY_EVT_INPUT_FORMAT so the channel strips relayout.
    @discardableResult
    func setI2SInputChannels(_ count: Int) -> UInt8 {
        if let status = typedPinStatus({ try $0.setI2sInputChannels(channels: UInt8(clamping: count)) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.i2sInputChannels = count } }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_I2S_INPUT_CHANNELS, value: UInt16(count), index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.i2sInputChannels = count }
            }
            return status
        }
        return 0xFF
    }

    /// Reads {current pipeline Hz, selected I2S Hz} via REQ_GET_INPUT_RATE.
    /// Only the selected I2S rate drives the picker; the current rate is valid
    /// for all sources but we already track it via REQ_GET_STATUS.
    func fetchInputRate() {
        if typedCore != nil { return }   // -- WP SW11 -- the core's mirror holds the selected rate
        guard let data = transport.getControlRequest(request: REQ_GET_INPUT_RATE, value: 0, index: 2, length: 8),
              data.count >= 8 else { return }
        let selected: UInt32 = data.withUnsafeBytes { $0.load(fromByteOffset: 4, as: UInt32.self) }
        DispatchQueue.main.async { self.i2sInputRateHz = selected }
    }

    /// Selects the I2S input sample rate (44100 / 48000 / 96000 Hz).  Applied
    /// live immediately if I2S is the active source.  Part of the device's
    /// output-config block (alongside the RX pins / MCK / BCK), so it only
    /// persists to flash via saveOutputConfig() in independent mode, or with
    /// the preset in with-preset mode - callers in Settings must mark the
    /// output config dirty (beginOutputEdit) so the save flow picks it up.
    /// Invalid rates are silently ignored by the firmware (no SET response).
    func setInputRate(_ hz: UInt32) {
        guard I2S_INPUT_RATES_HZ.contains(hz) else { return }
        DispatchQueue.main.async { self.i2sInputRateHz = hz }
        // -- WP SW11 -- The core takes the rate's index (0 = 44.1k, 1 = 48k, 2 = 96k).
        let index: UInt8 = hz == 44_100 ? 0 : hz == 96_000 ? 2 : 1
        if typedWrite({ try $0.setInputRate(index: index) }) { return }
        var le = hz.littleEndian
        let payload = withUnsafeBytes(of: &le) { Data($0) }
        transport.sendControlRequest(request: REQ_SET_INPUT_RATE, value: 0, index: 2, data: payload)
    }

    /// Sets the GPIO data pin for an I2S RX stereo pair (0..3).  Single IN
    /// transfer: wValue = (pair << 8) | gpio.  Returns the firmware PIN_CONFIG_*
    /// status code.  Pair 0 hot-swaps when stereo; any higher pair (or a
    /// multichannel config) restarts the input so every pair re-syncs.
    @discardableResult
    func setI2SRxPin(pair: Int = 0, _ pin: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setI2sRxPin(pair: UInt8(pair), pin: pin) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if self.i2sRxPins.indices.contains(pair) { self.i2sRxPins[pair] = pin }
                }
            }
            return status
        }
        let wValue = UInt16((pair << 8) | Int(pin))
        if let d = transport.getControlRequest(request: REQ_SET_I2S_RX_PIN, value: wValue, index: 2, length: 1),
           d.count >= 1 {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async {
                    if self.i2sRxPins.indices.contains(pair) { self.i2sRxPins[pair] = pin }
                }
            }
            return status
        }
        return 0xFF
    }
    // (The shared I2S BCK pin is set/read via setI2SBckPin/fetchI2SBckPin in the
    // I2S output section above — BCK is shared between I2S input and output.)

    // MARK: - I2S Clock-Slave Input Mode

    /// Reads the live I2S clock mode (0=master, 1=slave) via REQ_GET_I2S_CLOCK_MODE
    /// (0x89).  STALLs on firmware that predates the clock-slave feature, which
    /// leaves `i2sClockModeSupported` false so the mode picker hides itself.
    /// On success it also fetches the current slave-lock status.
    func fetchI2SClockMode() {
        // -- WP SW11 -- Through a core, the mode is its mirror's and the
        // feature its feature list's.
        if let core = typedCore {
            let generation = transport.generation
            let supported = coreFeature(core, "i2s_slave_clock")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.i2sClockModeSupported != supported { self.i2sClockModeSupported = supported }
            }
            if supported { fetchI2SSlaveStatus() }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_I2S_CLOCK_MODE, value: 0, index: 2, length: 1),
              d.count >= 1 else {
            DispatchQueue.main.async { self.i2sClockModeSupported = false }
            return
        }
        let mode = d[0]
        DispatchQueue.main.async {
            self.i2sClockModeSupported = true
            self.i2sClockMode = mode
        }
        fetchI2SSlaveStatus()
    }

    /// Selects the I2S clock mode (0=master, 1=slave) via REQ_SET_I2S_CLOCK_MODE
    /// (0x88), OUT 1 byte.  Fire-and-forget: the firmware defers the transition
    /// to its main loop and only applies it while I2S is the input source (it is
    /// otherwise recorded and applies at the next switch into I2S).  The live
    /// mode is confirmed via the input_config.i2s_clock_mode PARAM_CHANGED (or
    /// the first NOTIFY 0x09) - we optimistically mirror it here for a snappy UI.
    /// Part of the output-config block, so callers in Settings must mark it dirty
    /// (beginOutputEdit) for the save flow to pick it up.
    func setI2SClockMode(_ mode: UInt8) {
        let clamped: UInt8 = mode == I2S_CLOCK_MODE_SLAVE ? I2S_CLOCK_MODE_SLAVE : I2S_CLOCK_MODE_MASTER
        DispatchQueue.main.async { self.i2sClockMode = clamped }
        if typedWrite({ try $0.setI2sClockMode(mode: clamped) }) { return }   // -- WP SW11 --
        transport.sendControlRequest(request: REQ_SET_I2S_CLOCK_MODE, value: 0, index: 2, data: Data([clamped]))
    }

    /// Reads the 16-byte I2sSlaveStatusPacket via REQ_GET_I2S_SLAVE_STATUS (0x8A)
    /// for the clock-slave lock-state indicator (state, lock/loss counts, snapped
    /// + measured rates).  STALLs on firmware that predates the feature; the live
    /// mode byte it carries also refreshes `i2sClockMode`.
    func fetchI2SSlaveStatus() {
        // -- WP SW11 -- Through a core, its lock read (input 0).
        if let core = typedCore {
            let generation = transport.generation
            guard case .success(let lock) = core.call({ try $0.fetchInputLock(which: 0) }) else { return }
            var status = I2sSlaveStatus()
            status.state = lock.state
            status.clockMode = lock.clockMode
            status.lockCount = lock.lockCount
            status.lossCount = lock.lossCount
            status.detectedRate = lock.detectedRate
            status.measuredHz = lock.measuredHz
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                self.i2sClockModeSupported = true
                if self.i2sSlaveStatus != status { self.i2sSlaveStatus = status }
            }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_I2S_SLAVE_STATUS, value: 0, index: 2, length: 16),
              let status = I2sSlaveStatus.fromData(d) else { return }
        DispatchQueue.main.async {
            self.i2sClockModeSupported = true
            self.i2sSlaveStatus = status
            self.i2sClockMode = status.clockMode
        }
    }

    // MARK: - I2S Clock-Pin Mode (Unified / Split)

    /// Reads the live I2S clock-pin mode (0=unified, 1=split) via
    /// REQ_GET_I2S_CLOCK_PIN_MODE (0xFF).  STALLs on firmware that predates the
    /// feature, leaving `i2sClockPinModeSupported` false so the picker hides
    /// itself.  On success it also reads the stored slave-pair BCK pin (role 1)
    /// so the picker and pin-conflict checks reflect it even while dormant.
    func fetchI2SClockPinMode() {
        // -- WP SW11 -- Through a core, the mode and the slave pair's pin are
        // its mirror's and the feature its feature list's.
        if let core = typedCore {
            let generation = transport.generation
            let supported = coreFeature(core, "i2s_clock_pin_mode")
            DispatchQueue.main.async {
                guard self.transport.generation == generation else { return }
                if self.i2sClockPinModeSupported != supported { self.i2sClockPinModeSupported = supported }
            }
            return
        }
        guard let d = transport.getControlRequest(request: REQ_GET_I2S_CLOCK_PIN_MODE, value: 0, index: 2, length: 1),
              d.count >= 1 else {
            DispatchQueue.main.async { self.i2sClockPinModeSupported = false }
            return
        }
        let mode = d[0]
        DispatchQueue.main.async {
            self.i2sClockPinModeSupported = true
            self.i2sClockPinMode = mode
        }
        fetchI2SBckPin(role: I2S_BCK_ROLE_SLAVE)
    }

    /// Selects the I2S clock-pin mode (0=unified, 1=split) via
    /// REQ_SET_I2S_CLOCK_PIN_MODE (0xFE), IN transfer returning a PIN_CONFIG_*
    /// status.  Unlike the clock MODE (0x88) this applies synchronously - the
    /// status byte is authoritative (only the hardware restart is deferred).
    /// Part of the output-config block, so Settings callers must mark it dirty
    /// (beginOutputEdit) for the save flow to pick it up.
    @discardableResult
    func setI2SClockPinMode(_ mode: UInt8) -> UInt8 {
        if let status = typedPinStatus({ try $0.setI2sClockPinMode(mode: mode) }) {   // -- WP SW11 --
            if status == PIN_CONFIG_SUCCESS { DispatchQueue.main.async { self.i2sClockPinMode = mode } }
            return status
        }
        if let d = transport.getControlRequest(request: REQ_SET_I2S_CLOCK_PIN_MODE, value: UInt16(mode), index: 2, length: 1) {
            let status = d[0]
            if status == PIN_CONFIG_SUCCESS {
                DispatchQueue.main.async { self.i2sClockPinMode = mode }
            }
            return status
        }
        return 0xFF
    }

    // MARK: - External Control Interfaces (UART / I2C target)

    /// Probe the firmware for control-interface support and read the live UART/I2C
    /// configs plus the interface status.  REQ_GET_CTRL_IFACE_STATUS (0xF9) STALLs
    /// on firmware that predates this feature; if it returns nil,
    /// `controlInterfacesSupported` stays false and the Settings page hides itself.
    func fetchControlInterfaces() {
        if let core = typedCore { fetchControlInterfaces(core); return }   // -- WP SW11 --
        guard let s = transport.getControlRequest(request: REQ_GET_CTRL_IFACE_STATUS, value: 0, index: 2, length: 8),
              let status = CtrlIfaceStatus.fromData(s) else {
            DispatchQueue.main.async { self.controlInterfacesSupported = false }
            return
        }
        DispatchQueue.main.async {
            self.controlInterfacesSupported = true
            self.ctrlIfaceStatus = status
        }
        fetchUartCtrlConfig()
        fetchI2cCtrlConfig()
    }

    /// Read the live 8-byte UART control-interface config.
    func fetchUartCtrlConfig() {
        if let core = typedCore { fetchControlInterfaces(core); return }   // -- WP SW11 --
        guard let d = transport.getControlRequest(request: REQ_GET_UART_CONFIG, value: 0, index: 2, length: 8),
              let cfg = UartCtrlConfig.fromData(d) else { return }
        DispatchQueue.main.async { self.uartCtrlConfig = cfg }
    }

    /// Read the live 8-byte I2C control-interface config.
    func fetchI2cCtrlConfig() {
        if let core = typedCore { fetchControlInterfaces(core); return }   // -- WP SW11 --
        guard let d = transport.getControlRequest(request: REQ_GET_I2C_CONFIG, value: 0, index: 2, length: 8),
              let cfg = I2cCtrlConfig.fromData(d) else { return }
        DispatchQueue.main.async { self.i2cCtrlConfig = cfg }
    }

    /// Read just the interface status (last-apply outcome + live flags).  Cheap
    /// enough to poll after a SET to learn whether the peripheral came up.
    func fetchCtrlIfaceStatus() {
        if let core = typedCore { fetchControlInterfaces(core); return }   // -- WP SW11 --
        guard let d = transport.getControlRequest(request: REQ_GET_CTRL_IFACE_STATUS, value: 0, index: 2, length: 8),
              let status = CtrlIfaceStatus.fromData(d) else { return }
        DispatchQueue.main.async { self.ctrlIfaceStatus = status }
    }

    /// Apply and persist the UART control-interface config.  This is an OUT
    /// transfer carrying the full 8-byte struct; the firmware validates + applies
    /// it deferred on its main loop and (on success) writes flash, so the outcome
    /// is not available synchronously.  We give the device time to run the deferred
    /// apply, then read back the config and the PIN_CONFIG_* outcome from
    /// REQ_GET_CTRL_IFACE_STATUS (0xF9) and return the UART last-status byte.
    /// USB-only command; must be called off the main thread (it blocks).
    @discardableResult
    func setUartCtrlConfig(_ config: UartCtrlConfig) -> UInt8 {
        let generation = transport.generation
        if let core = typedCore {   // -- WP SW11 --
            let uart = UartConfig(enabled: config.enabled, txPin: config.txPin, rxPin: config.rxPin,
                                  notify: config.notifyEnable, baud: config.baud)
            return applyControlInterface(core, generation: generation) { try $0.setUart(config: uart) }?
                .uartLastStatus ?? 0xFF
        }
        transport.sendControlRequest(request: REQ_SET_UART_CONFIG, value: 0, index: 2, data: config.toData())
        // Deferred apply + ~45 ms flash blackout on the device; wait before reading.
        Thread.sleep(forTimeInterval: 0.25)
        // A device switch during the wait means the SET was dropped (or went
        // to the old device) - don't report the new device's status as ours.
        guard transport.generation == generation else { return 0xFF }
        fetchCtrlIfaceStatus()
        fetchUartCtrlConfig()
        if let d = transport.getControlRequest(request: REQ_GET_CTRL_IFACE_STATUS, value: 0, index: 2, length: 8),
           let status = CtrlIfaceStatus.fromData(d) {
            return status.uartLastStatus
        }
        return 0xFF
    }

    /// Apply and persist the I2C target control-interface config.  Same deferred
    /// OUT-then-readback flow as `setUartCtrlConfig`; returns the I2C last-status
    /// byte from REQ_GET_CTRL_IFACE_STATUS.  USB-only; call off the main thread.
    @discardableResult
    func setI2cCtrlConfig(_ config: I2cCtrlConfig) -> UInt8 {
        let generation = transport.generation
        if let core = typedCore {   // -- WP SW11 --
            let i2c = I2cConfig(enabled: config.enabled, sdaPin: config.sdaPin, sclPin: config.sclPin,
                                address: config.address)
            return applyControlInterface(core, generation: generation) { try $0.setI2c(config: i2c) }?
                .i2cLastStatus ?? 0xFF
        }
        transport.sendControlRequest(request: REQ_SET_I2C_CONFIG, value: 0, index: 2, data: config.toData())
        Thread.sleep(forTimeInterval: 0.25)
        // Same device-scoping as setUartCtrlConfig.
        guard transport.generation == generation else { return 0xFF }
        fetchCtrlIfaceStatus()
        fetchI2cCtrlConfig()
        if let d = transport.getControlRequest(request: REQ_GET_CTRL_IFACE_STATUS, value: 0, index: 2, length: 8),
           let status = CtrlIfaceStatus.fromData(d) {
            return status.i2cLastStatus
        }
        return 0xFF
    }

    // -- WP SW11 -- The control interfaces through a core.

    /// Its re-read of both configs and the status into its mirror, then
    /// published.  A device without them fails the status read.
    private func fetchControlInterfaces(_ core: CoreStateSource) {
        let generation = transport.generation
        let ok: Bool
        if case .success = core.call({ try $0.fetchCtrlIfaces() }) { ok = true } else { ok = false }
        let s = core.readState()
        publishControlInterfaces(s, supported: ok && s.ctrlIfaceSupported, generation: generation)
    }

    private func publishControlInterfaces(_ s: DspState, supported: Bool, generation: UInt64) {
        let uart = UartCtrlConfig(enabled: s.uart.enabled, txPin: s.uart.txPin, rxPin: s.uart.rxPin,
                                  notifyEnable: s.uart.notify, baud: s.uart.baud)
        let i2c = I2cCtrlConfig(enabled: s.i2c.enabled, sdaPin: s.i2c.sdaPin, sclPin: s.i2c.sclPin,
                                address: s.i2c.address)
        let status = CtrlIfaceStatus(uartLastStatus: s.ctrlStatus.uartLastStatus, uartLive: s.ctrlStatus.uartLive,
                                     i2cLastStatus: s.ctrlStatus.i2cLastStatus, i2cLive: s.ctrlStatus.i2cLive,
                                     protoVersion: s.ctrlStatus.protocolVersion)
        DispatchQueue.main.async {
            guard self.transport.generation == generation else { return }
            self.controlInterfacesSupported = supported
            guard supported else { return }
            self.ctrlIfaceStatus = status
            self.uartCtrlConfig = uart
            self.i2cCtrlConfig = i2c
        }
    }

    /// The core's setter, then, after the firmware's deferred apply, its
    /// re-read; the status after it, or nil when the write failed or the
    /// device changed meanwhile.  Blocking, as the raw path.
    private func applyControlInterface(_ core: CoreStateSource, generation: UInt64,
                                       _ body: (DspiCore) throws -> Void) -> CtrlIfaceStatus? {
        guard case .success = core.writeAndWait(body) else { return nil }
        Thread.sleep(forTimeInterval: 0.25)
        guard transport.generation == generation else { return nil }
        guard case .success = core.call({ try $0.fetchCtrlIfaces() }) else { return nil }
        let s = core.readState()
        publishControlInterfaces(s, supported: s.ctrlIfaceSupported, generation: generation)
        return CtrlIfaceStatus(uartLastStatus: s.ctrlStatus.uartLastStatus, uartLive: s.ctrlStatus.uartLive,
                               i2cLastStatus: s.ctrlStatus.i2cLastStatus, i2cLive: s.ctrlStatus.i2cLive,
                               protoVersion: s.ctrlStatus.protocolVersion)
    }

    // MARK: - Control Surfaces

    // -- WP SW12 -- The control surfaces through the core (SWIFT-SEAMS section
    // 3, "SW12"): the core keeps the configuration and performs every
    // operation (`csApply`, cs.rs); after each, its snapshot is shown
    // (`applyCsSnapshot`), so the Swift models and views stay as they are.
    // Without a core (USB without the flag, MIDI, a Link client) the raw
    // requests below run as before.

    /// One `csApply` operation on the core, then its snapshot shown.  A
    /// write queues behind the writes sent before it and publishes the
    /// journal (the core records every `cs.*` operation and Undo steps over
    /// it: the editor's own Revert is the way back); a read does neither.
    /// Blocking: call off the main thread.
    @discardableResult
    private func csCoreOp(_ core: CoreStateSource, _ op: [String: Any], write: Bool) -> CsCoreReply {
        let text = CsCoreJSON.text(op)
        let body: (DspiCore) -> (String, String) = { ($0.csApply(op: text), $0.csSnapshot()) }
        let r = write ? core.writeAndWait(body) : core.call(body)
        guard case .success(let (text, snapshot)) = r else { return CsCoreReply("{}") }
        var reply = CsCoreReply(text)
        reply.snapshot = CsCoreSnapshot(snapshot)
        publishCsSnapshot(reply.snapshot ?? CsCoreSnapshot())
        // -- WP SW12 -- fixup sw12 F2: a write the core refused before the
        // device answered (read-only, locked, denied, unsupported, transfer)
        // says why, as a typed write's refusal does (`writeRefused`).
        if write, let refusal = reply.refusal { core.postRefusal(refusal.message, resync: refusal.resync) }
        return reply
    }

    /// The core's last read, shown: no device I/O.  Blocking (the core's
    /// queue): call off the main thread.
    @discardableResult
    private func csShowSnapshot(_ core: CoreStateSource) -> CsCoreSnapshot {
        let s = (try? core.call { $0.csSnapshot() }.get()).map(CsCoreSnapshot.init) ?? CsCoreSnapshot()
        publishCsSnapshot(s)
        return s
    }

    private func publishCsSnapshot(_ s: CsCoreSnapshot) {
        DispatchQueue.main.async { self.applyCsSnapshot(s) }
    }

    /// Main thread: the core's snapshot into the published control-surface
    /// state, only what changed (a view holding the rest does not redraw).
    func applyCsSnapshot(_ s: CsCoreSnapshot) {
        func set<T: Equatable>(_ kp: ReferenceWritableKeyPath<DSPViewModel, T>, _ v: T) {
            if self[keyPath: kp] != v { self[keyPath: kp] = v }
        }
        set(\.controlSurfacesSupported, s.supported)
        guard s.supported else { return }
        set(\.csCaps, s.caps)
        set(\.csNounDescs, s.nouns)
        set(\.csStatus, s.status)
        set(\.csBindings, s.bindings)
        set(\.csNames, s.names)
        set(\.csIrCommands, s.ir)
        set(\.csGroups, s.groups)
        set(\.csMacros, s.macros)
        set(\.csExtStatus, s.ext)
        set(\.csDisplayCfg, s.displayCfg)
        set(\.csDisplayPages, s.displayPages)
        set(\.csDisplayStatus, s.displayStatus)
        set(\.csDisplayMaxPages, s.displayMaxPages)
        set(\.csDisplayModelCount, s.displayModelCount)
        set(\.csAuxState, s.auxState)
        set(\.csAuxLevel, s.auxLevel)
    }

    /// Enumerate Control Surfaces capabilities + live state at connect.  The
    /// caps header (REQ_GET_CS_CAPS, wValue=0xFFFF) doubles as the feature
    /// probe: older firmware STALLs it and the Settings page hides itself.  On
    /// success we cache the type table, every per-noun descriptor, all live
    /// bindings, and the status packet, so the picker UI is built entirely from
    /// device-served tables (spec §4/§8.1).
    func fetchControlSurfaces() {
        if let core = typedCore {   // -- WP SW12 -- csFetch, then the snapshot
            let r = core.call { c -> String in try c.csFetch(); return c.csSnapshot() }
            publishCsSnapshot((try? r.get()).map(CsCoreSnapshot.init) ?? CsCoreSnapshot())
            return
        }
        // Ask for more than the largest known header: the type table grows as
        // component types are added (40 bytes at caps v3-v9, 44 at v10 with
        // CS_TYPE_DISPLAY), and a request pinned to one version silently
        // truncates the tail that carries the IR, group and macro counts - so
        // those features would read as absent on newer firmware.  The parser
        // locates that tail from `type_count`, never a fixed offset.
        guard let h = transport.getControlRequest(request: REQ_GET_CS_CAPS, value: CS_CAPS_ALL, index: 2, length: 64),
              let caps = CsCapsHeader.fromData(h) else {
            DispatchQueue.main.async { self.controlSurfacesSupported = false }
            return
        }
        // Per-noun descriptors (kind, enum count, unit-encoded range, unit,
        // target addressing, accepted actions).  The picker indexes this array
        // by noun number, so a failed read must leave a hole rather than close
        // the gap: skipping would shift every later noun down one and silently
        // hand the UI the wrong unit, range, and action mask for the rest of
        // the table.  An all-zero descriptor reads as actions == 0, which is
        // already the "unavailable on this platform" encoding (ADAT on RP2040),
        // so the noun simply drops out of the menus.
        var nouns: [CsNounDesc] = []
        for n in 0..<Int(caps.nounCount) {
            let d = transport.getControlRequest(request: REQ_GET_CS_CAPS, value: UInt16(n), index: 2, length: 12)
            nouns.append(d.flatMap(CsNounDesc.fromData) ?? CsNounDesc())
        }
        DispatchQueue.main.async {
            self.controlSurfacesSupported = true
            self.csCaps = caps
            self.csNounDescs = nouns
        }
        fetchCsStatus()
        for slot in 0..<CS_MAX_BINDINGS {
            fetchCsBinding(slot: slot)
            fetchCsName(slot: slot)
        }
        // IR command sub-slots (only populated when a receiver is configured,
        // but always safe to read - an empty sub-slot reads back all-zero).
        if caps.maxIrCommands > 0 {
            for sub in 0..<min(Int(caps.maxIrCommands), CS_MAX_IR_COMMANDS) {
                fetchCsIrCommand(sub: sub)
            }
        }
        // Target groups and macros (caps v9).  The counts sit in the caps
        // header's formerly-reserved tail, so a pre-v9 device reports 0 here
        // and these loops simply do not run.
        for g in 0..<min(Int(caps.maxGroups), CS_MAX_GROUPS) { fetchCsGroup(g) }
        for m in 0..<min(Int(caps.maxMacros), CS_MAX_MACROS) { fetchCsMacro(m) }
        if caps.maxGroups > 0 || caps.maxMacros > 0 { fetchCsExtStatus() }
        // I2C display (caps v10).  The config GET carries the page limit, so it
        // has to land before the page loop.
        if caps.typeCount > UInt8(CS_TYPE_DISPLAY) {
            let pages = fetchCsDisplayCfg()
            for i in 0..<min(Int(pages), CS_MAX_DISPLAY_PAGES) { fetchCsDisplayPage(i) }
            fetchCsDisplayStatus()
        }
        // Auxiliary outputs (caps v18): the component types are the gate.
        if caps.typeCount > UInt8(CS_TYPE_AUX_PWM) {
            fetchCsAuxValues()
        }
        // Runs after every fetch above (all dispatch to main in FIFO order): if
        // the device is clean, snapshot the loaded config as the saved baseline
        // so csDirty can tell net-zero edits from real ones.
        DispatchQueue.main.async {
            if !self.csStatus.dirty { self.captureCsCleanSnapshot() }
        }
    }

    /// Read the live 24-byte binding for one slot into `csBindings[slot]`.
    func fetchCsBinding(slot: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard slot >= 0, slot < CS_MAX_BINDINGS,
              let d = transport.getControlRequest(request: REQ_GET_CS_BINDING, value: UInt16(slot), index: 2, length: 24),
              let bind = CsBinding.fromData(d) else { return }
        DispatchQueue.main.async {
            if slot < self.csBindings.count { self.csBindings[slot] = bind }
        }
    }

    /// Read the device-persistent name for one slot into `csNames[slot]`
    /// (spec §3.4).  Synchronous read of the directory RAM cache; always 32
    /// bytes, NUL-terminated.  Empty when the slot is unnamed.
    func fetchCsName(slot: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard slot >= 0, slot < CS_MAX_BINDINGS,
              let d = transport.getControlRequest(request: REQ_GET_CS_NAME, value: UInt16(slot), index: 2, length: UInt16(CS_NAME_LEN)) else { return }
        let name = String(decoding: d.prefix { $0 != 0 }, as: UTF8.self)
        DispatchQueue.main.async {
            if slot < self.csNames.count { self.csNames[slot] = name }
        }
    }

    /// Read one live 16-byte IR command sub-slot into `csIrCommands[sub]`.
    func fetchCsIrCommand(sub: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard sub >= 0, sub < CS_MAX_IR_COMMANDS,
              let d = transport.getControlRequest(request: REQ_GET_CS_IR_CMD, value: UInt16(sub), index: 2, length: 16),
              let cmd = IrCommand.fromData(d) else { return }
        DispatchQueue.main.async {
            if sub < self.csIrCommands.count { self.csIrCommands[sub] = cmd }
        }
    }

    /// Read the status packet (last-apply outcome, dirty flag, active mask,
    /// per-slot health, IR tail).  Cheap enough to poll after a SET.  We always
    /// ask for the v6 length; older firmware short-reads and CsStatusPacket
    /// parses whichever layout comes back.
    func fetchCsStatus() {
        if let core = typedCore { csCoreOp(core, ["op": "refreshStatus"], write: false); return }   // -- WP SW12 --
        guard let d = transport.getControlRequest(request: REQ_GET_CS_STATUS, value: 0, index: 2, length: CS_STATUS_LEN),
              let st = CsStatusPacket.fromData(d) else { return }
        DispatchQueue.main.async { self.csStatus = st }
    }

    /// Poll REQ_GET_CS_STATUS until a deferred SET/save/revert resolves, i.e.
    /// `lastSlot == expectedSlot` and `lastStatus != CS_STATUS_PENDING`, or the
    /// ~500 ms budget (25 x 20 ms) runs out.  Returns the final status code
    /// (0xFF if no readback ever succeeded).  USB-only; blocks - call off-main.
    private func pollCsDeferred(expectedSlot: UInt8) -> UInt8 {
        let generation = transport.generation
        var result: UInt8 = CS_STATUS_PENDING
        for _ in 0..<25 {
            Thread.sleep(forTimeInterval: 0.02)
            // Stop polling if a device switch lands mid-wait - the new
            // device's status says nothing about our deferred apply.
            guard transport.generation == generation else { return 0xFF }
            guard let d = transport.getControlRequest(request: REQ_GET_CS_STATUS, value: 0, index: 2, length: CS_STATUS_LEN),
                  let st = CsStatusPacket.fromData(d) else { continue }
            if st.lastSlot == expectedSlot && st.lastStatus != CS_STATUS_PENDING {
                result = st.lastStatus
                break
            }
        }
        return result
    }

    /// Apply one Control Surface binding to `slot` (live-only preview; spec §3.2
    /// / §3.5).  SET (0x84) is an OUT transfer carrying the 24-byte binding; the
    /// firmware validates + applies it deferred on its main loop and marks the
    /// live config dirty on success - nothing reaches flash until `csSave()`.
    /// The outcome is not synchronous: we poll REQ_GET_CS_STATUS (0x87) until
    /// `lastSlot == slot` leaves CS_STATUS_PENDING, then refresh the live
    /// binding + status.  Returns the PIN_CONFIG_* / CS_STATUS_* result (0xFF if
    /// readback failed).  USB-only; must be called off the main thread (blocks).
    @discardableResult
    func setCsBinding(slot: Int, binding: CsBinding) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core waits for the deferred apply and re-reads
            return csCoreOp(core, ["op": "binding", "index": slot, "binding": CsCoreJSON.binding(binding)],
                            write: true).statusByte
        }
        transport.sendControlRequest(request: REQ_SET_CS_BINDING, value: UInt16(slot), index: 2, data: binding.toData())
        let result = pollCsDeferred(expectedSlot: UInt8(slot))
        fetchCsBinding(slot: slot)
        // An aux slot that appeared, changed kind or went away moves its live
        // on/off flag and level (a same-type edit keeps them; aux spec 4).
        fetchCsAuxValues()
        fetchCsStatus()
        return result
    }

    /// Set (or clear) a slot's name (spec §3.4).  SET (0x8B) is an OUT transfer
    /// applied deferred like a binding: it's a live-only preview that sets the
    /// device `dirty` flag, so the change takes effect immediately but only
    /// reaches flash on csSave() and is undone by csRevert().  Send a single NUL
    /// byte to clear.  Returns the PIN_CONFIG_* / CS_STATUS_* result.  USB-only;
    /// must be called off the main thread (blocks on the poll).
    @discardableResult
    func setCsName(slot: Int, name: String) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 --
            return csCoreOp(core, ["op": "name", "index": slot, "name": CsCoreJSON.name(name)], write: true).statusByte
        }
        // Truncate to 31 characters + implicit NUL (the firmware truncates too).
        var bytes = Array(name.utf8.prefix(CS_NAME_LEN - 1))
        if bytes.isEmpty { bytes = [0] }   // empty payload is INVALID_VALUE; one NUL clears
        transport.sendControlRequest(request: REQ_SET_CS_NAME, value: UInt16(slot), index: 2, data: Data(bytes))
        let result = pollCsDeferred(expectedSlot: UInt8(slot))
        fetchCsName(slot: slot)
        fetchCsStatus()
        return result
    }

    /// Apply one IR command sub-slot (live-only preview; spec §3.6).  SET (0x8D)
    /// carries the 16-byte IrCommand; the outcome is reported with
    /// `lastSlot == 0x80 | sub`.  Send an all-zero record to clear.  Returns the
    /// status code.  USB-only; must be called off the main thread (blocks).
    @discardableResult
    func setCsIrCommand(sub: Int, command: IrCommand) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 --
            return csCoreOp(core, ["op": "ir", "index": sub, "command": CsCoreJSON.ir(command)], write: true).statusByte
        }
        transport.sendControlRequest(request: REQ_SET_CS_IR_CMD, value: UInt16(sub), index: 2, data: command.toData())
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_IR_FLAG | UInt8(sub))
        fetchCsIrCommand(sub: sub)
        fetchCsStatus()
        return result
    }

    /// Persist the whole live Control Surfaces config (16 bindings + 16 IR
    /// commands) in one flash write and clear `dirty` (REQ_CS_SAVE, 0x9D; spec
    /// §3.5).  Deferred, reported with `lastSlot == 0xFF`.  Returns the status
    /// code.  USB-only; must be called off the main thread (blocks).
    @discardableResult
    func csSave() -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core re-reads all and rebases its clean copy
            return csCoreOp(core, ["op": "save"], write: true).statusByte
        }
        _ = transport.getControlRequest(request: REQ_CS_SAVE, value: 0, index: 2, length: 1)
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_SAVE)
        fetchCsStatus()
        // The live config is now the saved config: rebase the clean baseline so
        // subsequent net-zero churn doesn't re-strand the banner.
        if result == PIN_CONFIG_SUCCESS {
            // A save folds the live state and level into the boot fields of
            // every boot-saved aux slot's binding (aux spec 4), so those
            // records changed underneath us.  Re-read the bindings before
            // rebasing or the next refresh would read as an unsaved edit.
            for slot in 0..<CS_MAX_BINDINGS { fetchCsBinding(slot: slot) }
            DispatchQueue.main.async { self.captureCsCleanSnapshot() }
        }
        return result
    }

    /// Discard the live preview by reloading the stored config (REQ_CS_REVERT,
    /// 0x9E; spec §3.5), then clear `dirty`.  Deferred, reported with
    /// `lastSlot == 0xFF`.  Refreshes every binding + IR command afterward so
    /// the UI reflects the restored state.  Returns the status code.  USB-only;
    /// must be called off the main thread (blocks).
    @discardableResult
    func csRevert() -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core re-reads all and rebases its clean copy
            return csCoreOp(core, ["op": "revert"], write: true).statusByte
        }
        _ = transport.getControlRequest(request: REQ_CS_REVERT, value: 0, index: 2, length: 1)
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_SAVE)
        for slot in 0..<CS_MAX_BINDINGS {
            fetchCsBinding(slot: slot)
            fetchCsName(slot: slot)   // revert restores the stored names too (spec 3.4)
        }
        for sub in 0..<CS_MAX_IR_COMMANDS { fetchCsIrCommand(sub: sub) }
        // Revert restores the stored groups and macros as well, and cancels a
        // running macro, so re-read both tables and the sequencer state.  Loops
        // run to the wire maximum rather than the caps count, like the two
        // above: `csCaps` is main-thread state and this runs off it, and a
        // pre-v9 device simply STALLs each GET and leaves the table alone.
        for g in 0..<CS_MAX_GROUPS { fetchCsGroup(g) }
        for m in 0..<CS_MAX_MACROS { fetchCsMacro(m) }
        fetchCsExtStatus()
        // The display config and pages are covered by the same revert.
        let pages = fetchCsDisplayCfg()
        for i in 0..<min(Int(pages), CS_MAX_DISPLAY_PAGES) { fetchCsDisplayPage(i) }
        fetchCsDisplayStatus()
        // Revert carries each aux output's live value across when its slot
        // reloads with the same type, but a slot whose type changed comes up at
        // its boot value, so the live picture has to be re-read (aux spec 4).
        fetchCsAuxValues()
        fetchCsStatus()
        // Live now mirrors flash again; rebase the clean baseline after the
        // re-fetches above land (FIFO on main).
        DispatchQueue.main.async { self.captureCsCleanSnapshot() }
        return result
    }

    // MARK: - Control Surfaces: auxiliary outputs (caps v18)
    //
    // An aux output is a CS_TYPE_AUX_OUT / CS_TYPE_AUX_PWM binding: its pin,
    // name and boot behaviour go through the ordinary binding and name SETs.
    // Only the two live values have commands of their own, applied in the
    // firmware's handler with no flash write.  See control_surfaces_aux_spec.md.

    /// Read every live state and level in one transfer (0x05 with wValue
    /// 0xFFFF): sixteen state bytes, then sixteen little-endian 8.8 levels.
    /// Slots that are not an up aux output read zero (aux spec 3.3).  A
    /// pre-v18 device STALLs and the arrays keep their zeros.
    func fetchCsAuxValues() {
        if let core = typedCore { csCoreOp(core, ["op": "auxRead"], write: false); return }   // -- WP SW12 --
        guard let d = transport.getControlRequest(request: REQ_GET_CS_AUX_STATE, value: CS_AUX_STATE_ALL, index: 2, length: 48),
              d.count >= 48 else { return }
        let b = d.startIndex
        let states = (0..<CS_MAX_BINDINGS).map { d[b + $0] != 0 }
        let levels = (0..<CS_MAX_BINDINGS).map { i -> UInt16 in
            let off = b + CS_MAX_BINDINGS + 2 * i
            return min(UInt16(d[off]) | (UInt16(d[off + 1]) << 8), CS_AUX_LEVEL_MAX_Q8)
        }
        DispatchQueue.main.async {
            self.csAuxState = states
            self.csAuxLevel = levels
        }
    }

    /// Switch one aux output on or off.  Applied in the firmware's handler with
    /// no flash write and no dirty flag, so this is a plain fire-and-forget SET
    /// like a volume change; the device echoes it back on NOTIFY_EVT_CS_AUX.
    func setCsAuxState(_ slot: Int, on: Bool) {
        guard slot >= 0, slot < CS_MAX_BINDINGS else { return }
        self.csAuxState[slot] = on
        guard csAuxSupported else { return }
        if let core = typedCore {   // -- WP SW12 -- queued; a refusal puts the device's value back
            csQueuedWrite(core, lane: "cs.aux.\(slot)", ["op": "auxState", "index": slot, "on": on])
            return
        }
        transport.sendControlRequest(request: REQ_SET_CS_AUX_STATE, value: UInt16(slot), index: 2,
                               data: Data([on ? 1 : 0]))
    }

    /// Set a dimmable output's level in percent.  8.8 on the wire, clamped to
    /// 100 %; immediate, like the state above.
    func setCsAuxLevel(_ slot: Int, percent: Float) {
        guard slot >= 0, slot < CS_MAX_BINDINGS else { return }
        let q8 = UInt16(min(Float(CS_AUX_LEVEL_MAX_Q8), max(0, (percent * 256).rounded())))
        self.csAuxLevel[slot] = q8
        guard csAuxSupported else { return }
        if let core = typedCore {   // -- WP SW12 -- a drag's newest level replaces one still queued
            csQueuedWrite(core, lane: "cs.aux.level.\(slot)",
                       ["op": "auxLevel", "index": slot, "percent": Double(q8) / 256])
            return
        }
        transport.sendControlRequest(request: REQ_SET_CS_AUX_LEVEL, value: UInt16(slot), index: 2,
                               data: Data([UInt8(q8 & 0xFF), UInt8(q8 >> 8)]))
    }

    /// -- WP SW12 -- A control-surface write queued on the core's queue
    /// (main thread: it does not wait); `lane`: a newer value for the same
    /// key replaces one still queued (an aux level drag).  The value shown is
    /// the one sent; when the core or the device refuses it, the device's
    /// values are shown again (an applied one changes nothing, so a drag never
    /// jumps back), and a refusal before the device says why (fixup sw12 F2).
    private func csQueuedWrite(_ core: CoreStateSource, lane: String?, _ op: [String: Any]) {
        let text = CsCoreJSON.text(op)
        core.write(lane: lane) { [weak self, weak core] c in
            let reply = CsCoreReply(c.csApply(op: text))
            guard !reply.ok else { return }
            self?.publishCsSnapshot(CsCoreSnapshot(c.csSnapshot()))
            if let refusal = reply.refusal { core?.postRefusal(refusal.message, resync: refusal.resync) }
        }
    }

    // MARK: - Control Surfaces: target groups and macros (caps v9)

    /// Read one live 40-byte group record into `csGroups[index]`.
    func fetchCsGroup(_ index: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard index >= 0, index < CS_MAX_GROUPS,
              let d = transport.getControlRequest(request: REQ_GET_CS_GROUP, value: UInt16(index), index: 2, length: CS_GROUP_LEN),
              let group = CsGroup.fromData(d) else { return }
        DispatchQueue.main.async {
            if index < self.csGroups.count { self.csGroups[index] = group }
        }
    }

    /// Read one live 132-byte macro (name, step count and all 8 steps) into
    /// `csMacros[index]`.  The GET returns the whole record even though a SET
    /// has to be split into a header plus one write per step.
    func fetchCsMacro(_ index: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard index >= 0, index < CS_MAX_MACROS,
              let d = transport.getControlRequest(request: REQ_GET_CS_MACRO, value: UInt16(index), index: 2, length: CS_MACRO_LEN),
              let macro = CsMacro.fromData(d) else { return }
        DispatchQueue.main.async {
            if index < self.csMacros.count { self.csMacros[index] = macro }
        }
    }

    /// Read the 24-byte extended status: group and macro record validity plus
    /// what the sequencer is doing right now (groups+macros spec §2.6).
    /// Returns the packet as well as publishing it, so a caller polling the
    /// sequencer can test the result directly instead of reading back the
    /// @Published copy it just scheduled onto the main thread.
    @discardableResult
    func fetchCsExtStatus() -> CsExtStatusPacket? {
        if let core = typedCore {   // -- WP SW12 -- the sequencer's live state: the core re-reads the status
            let r = csCoreOp(core, ["op": "refreshStatus"], write: false)
            guard r.ok else { return nil }
            return r.snapshot?.ext   // fixup sw12 N3: one round trip
        }
        guard let d = transport.getControlRequest(request: REQ_GET_CS_EXT_STATUS, value: 0, index: 2, length: CS_EXT_STATUS_LEN),
              let st = CsExtStatusPacket.fromData(d) else { return nil }
        DispatchQueue.main.async { self.csExtStatus = st }
        return st
    }

    /// Apply one target group (live-only preview).  Send an all-zero record to
    /// clear the slot.  The outcome is reported with `lastSlot == 0x40 | group`.
    /// Applying a group re-validates every binding that references one, so the
    /// binding slot statuses are re-read too: emptying a group in use
    /// deactivates its dependants rather than being refused (spec §4.4).
    /// Returns the status code.  USB-only; must be called off the main thread.
    @discardableResult
    func setCsGroup(_ index: Int, group: CsGroup) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core re-reads the group, the bindings and the status
            return csCoreOp(core, ["op": "group", "index": index, "group": CsCoreJSON.group(group)], write: true).statusByte
        }
        transport.sendControlRequest(request: REQ_SET_CS_GROUP, value: UInt16(index), index: 2, data: group.toData())
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_GROUP_FLAG | UInt8(index))
        fetchCsGroup(index)
        for slot in 0..<CS_MAX_BINDINGS { fetchCsBinding(slot: slot) }
        fetchCsStatus()
        fetchCsExtStatus()
        return result
    }

    /// Apply one macro step (live-only preview).  All-zero clears the step.
    /// wValue packs the step index above the macro index.  Reported with
    /// `lastSlot == 0x60 | macro`, the same tag as the header SET.
    @discardableResult
    func setCsMacroStep(macro: Int, step: Int, value: CsMacroStep) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core writes a whole macro: this one with the step changed
            guard step >= 0, step < CS_MAX_MACRO_STEPS else { return CS_STATUS_INVALID_MACRO }
            let macros = csShowSnapshot(core).macros
            guard macros.indices.contains(macro) else { return CS_STATUS_INVALID_MACRO }
            var m = macros[macro]
            // fixup sw12 N4: the step count stays the header's.  A slot past
            // it is not run (dead data) and the core keeps such slots clear,
            // so there is nothing to write for one.
            guard step < Int(m.stepCount) else { return PIN_CONFIG_SUCCESS }
            m.steps[step] = value
            return csCoreOp(core, ["op": "macro", "index": macro, "macro": CsCoreJSON.macro(m)], write: true).statusByte
        }
        let wValue = UInt16(step << 8) | UInt16(macro)
        transport.sendControlRequest(request: REQ_SET_CS_MACRO_STEP, value: wValue, index: 2, data: value.toData())
        return pollCsDeferred(expectedSlot: CS_LAST_SLOT_MACRO_FLAG | UInt8(macro))
    }

    /// Apply one macro's name and step count (the 36-byte header).
    @discardableResult
    func setCsMacroHeader(_ index: Int, macro: CsMacro) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the name and step count over the core's steps
            let macros = csShowSnapshot(core).macros
            guard macros.indices.contains(index) else { return CS_STATUS_INVALID_MACRO }
            var m = macros[index]
            m.name = macro.name
            m.stepCount = min(macro.stepCount, UInt8(CS_MAX_MACRO_STEPS))
            return csCoreOp(core, ["op": "macro", "index": index, "macro": CsCoreJSON.macro(m)], write: true).statusByte
        }
        transport.sendControlRequest(request: REQ_SET_CS_MACRO, value: UInt16(index), index: 2, data: macro.headerData())
        return pollCsDeferred(expectedSlot: CS_LAST_SLOT_MACRO_FLAG | UInt8(index))
    }

    /// Apply a whole macro: every step first, then the header carrying the
    /// final step count.  That order matters - the spec's recommendation exists
    /// because a macro fired mid-edit must never see a `step_count` reaching
    /// past the steps actually written.  Returns the first failing status, or
    /// success.  USB-only; must be called off the main thread (blocks).
    @discardableResult
    func setCsMacro(_ index: Int, macro: CsMacro) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- steps, then the header, under the device lock: the core's one op
            return csCoreOp(core, ["op": "macro", "index": index, "macro": CsCoreJSON.macro(macro)], write: true).statusByte
        }
        var worst: UInt8 = PIN_CONFIG_SUCCESS
        for (s, step) in macro.steps.enumerated() where s < CS_MAX_MACRO_STEPS {
            let r = setCsMacroStep(macro: index, step: s, value: step)
            if r != PIN_CONFIG_SUCCESS && worst == PIN_CONFIG_SUCCESS { worst = r }
        }
        let r = setCsMacroHeader(index, macro: macro)
        if r != PIN_CONFIG_SUCCESS && worst == PIN_CONFIG_SUCCESS { worst = r }
        fetchCsMacro(index)
        fetchCsStatus()
        fetchCsExtStatus()
        return worst
    }

    /// Fire a macro now, or cancel the running one with
    /// `CS_MACRO_FIRE_CANCEL`.  Immediate, not deferred: the reply is the
    /// accept/reject status, and the sequence then runs on the device.  Firing
    /// while another macro runs cancels that one at its current step boundary.
    @discardableResult
    func csMacroFire(_ index: UInt16) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core re-reads the status (and the sequencer) after it
            let op: [String: Any] = index == CS_MACRO_FIRE_CANCEL ? ["op": "macroCancel"] : ["op": "macroFire", "index": index]
            let r = csCoreOp(core, op, write: true)
            return r.ok || r.status != nil ? r.statusByte : CS_STATUS_INVALID_MACRO
        }
        let r = transport.getControlRequest(request: REQ_CS_MACRO_FIRE, value: index, index: 2, length: 1)
        fetchCsExtStatus()
        return r?.first ?? CS_STATUS_INVALID_MACRO
    }

    // MARK: - Control Surfaces: I2C display (caps v10)

    /// Read the display config, returning the device's page-slot count so the
    /// caller can size its page loop.  The 16-byte response is a 4-byte limits
    /// header followed by the 12-byte config.
    @discardableResult
    func fetchCsDisplayCfg() -> UInt8 {
        if let core = typedCore { return csShowSnapshot(core).displayMaxPages }   // -- WP SW12 -- the core's copy
        guard let d = transport.getControlRequest(request: REQ_GET_CS_DISPLAY_CFG, value: 0, index: 2,
                                            length: CS_DISPLAY_CFG_GET_LEN),
              d.count >= Int(CS_DISPLAY_CFG_GET_LEN),
              let cfg = CsDisplayCfg.fromData(d, offset: 4) else { return 0 }
        let b = d.startIndex
        let maxPages = d[b], modelCount = d[b + 1]
        DispatchQueue.main.async {
            self.csDisplayMaxPages = maxPages
            self.csDisplayModelCount = modelCount
            self.csDisplayCfg = cfg
        }
        return maxPages
    }

    /// Read one 4-byte page record into `csDisplayPages[index]`.
    func fetchCsDisplayPage(_ index: Int) {
        if let core = typedCore { csShowSnapshot(core); return }   // -- WP SW12 -- the core's copy
        guard index >= 0, index < CS_MAX_DISPLAY_PAGES,
              let d = transport.getControlRequest(request: REQ_GET_CS_DISPLAY_PAGE, value: UInt16(index),
                                            index: 2, length: CS_DISPLAY_PAGE_LEN),
              let page = CsDisplayPage.fromData(d) else { return }
        DispatchQueue.main.async {
            if index < self.csDisplayPages.count { self.csDisplayPages[index] = page }
        }
    }

    /// Read the live display state: whether the panel came up, what it shows,
    /// and the running I2C abort count.
    @discardableResult
    func fetchCsDisplayStatus() -> CsDisplayStatus? {
        if let core = typedCore {   // -- WP SW12 -- the panel's live state, read by the core
            let r = csCoreOp(core, ["op": "displayStatus"], write: false)
            guard r.ok else { return nil }
            return r.snapshot?.displayStatus   // fixup sw12 N3: one round trip
        }
        guard let d = transport.getControlRequest(request: REQ_GET_CS_DISPLAY_STATUS, value: 0, index: 2,
                                            length: CS_DISPLAY_STATUS_LEN),
              let st = CsDisplayStatus.fromData(d) else { return nil }
        DispatchQueue.main.async { self.csDisplayStatus = st }
        return st
    }

    /// Apply the display config (live-only preview; reported with `lastSlot`
    /// 0x50).  USB-only; must be called off the main thread (blocks on poll).
    ///
    /// Serialized with the page SETs below: the firmware tags the config bare
    /// and pages `0x50 | page`, so the config and page 0 arrive under the same
    /// byte.  One display SET at a time keeps that from being ambiguous, and
    /// these are auto-applied from the UI as the user edits, which is exactly
    /// the case that would otherwise overlap.
    @discardableResult
    func setCsDisplayCfg(_ cfg: CsDisplayCfg) -> UInt8 {
        displaySetQueue.sync { performDisplayCfgSet(cfg) }
    }

    /// The write itself.  Assumes the caller already holds `displaySetQueue`;
    /// `setCsDisplayCfg` takes it, and `queueDisplayCfg` runs on it.
    private func performDisplayCfgSet(_ cfg: CsDisplayCfg) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 -- the core re-reads the display and the status
            let result = csCoreOp(core, ["op": "displayConfig", "config": CsCoreJSON.displayConfig(cfg)],
                                  write: true).statusByte
            DispatchQueue.main.async { self.csDisplayLastStatus = result }
            return result
        }
        transport.sendControlRequest(request: REQ_SET_CS_DISPLAY_CFG, value: 0, index: 2, data: cfg.toData())
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_DISPLAY_FLAG)
        fetchCsDisplayCfg()
        fetchCsStatus()
        fetchCsDisplayStatus()
        // These apply live as the user edits, so nothing waits on a return
        // value; publish the outcome or a rejection would be invisible - the
        // row would just snap back when the re-fetch landed.
        DispatchQueue.main.async { self.csDisplayLastStatus = result }
        return result
    }

    /// Queue a config write, replacing any that has not gone out yet.  These
    /// come from live UI edits (a scroll wheel commits per detent), so sending
    /// every intermediate value would be a queue of stale round trips.
    func queueDisplayCfg(_ cfg: CsDisplayCfg) {
        displayPendingLock.lock(); pendingDisplayCfg = cfg; displayPendingLock.unlock()
        displaySetQueue.async { [weak self] in
            guard let self else { return }
            self.displayPendingLock.lock()
            let next = self.pendingDisplayCfg
            self.pendingDisplayCfg = nil
            self.displayPendingLock.unlock()
            guard let next else { return }   // a later edit already carried it
            _ = self.performDisplayCfgSet(next)
        }
    }

    /// Same for one page slot; edits to different pages do not displace each other.
    func queueDisplayPage(_ index: Int, _ page: CsDisplayPage) {
        displayPendingLock.lock(); pendingDisplayPages[index] = page; displayPendingLock.unlock()
        displaySetQueue.async { [weak self] in
            guard let self else { return }
            self.displayPendingLock.lock()
            let next = self.pendingDisplayPages.removeValue(forKey: index)
            self.displayPendingLock.unlock()
            guard let next else { return }
            _ = self.performDisplayPageSet(index, page: next)
        }
    }

    /// Apply one page (all-zero clears it), reported with `lastSlot` 0x50 | page.
    @discardableResult
    func setCsDisplayPage(_ index: Int, page: CsDisplayPage) -> UInt8 {
        displaySetQueue.sync { performDisplayPageSet(index, page: page) }
    }

    private func performDisplayPageSet(_ index: Int, page: CsDisplayPage) -> UInt8 {
        if let core = typedCore {   // -- WP SW12 --
            let result = csCoreOp(core, ["op": "displayPage", "index": index, "page": CsCoreJSON.page(page)],
                                  write: true).statusByte
            DispatchQueue.main.async { self.csDisplayLastStatus = result }
            return result
        }
        transport.sendControlRequest(request: REQ_SET_CS_DISPLAY_PAGE, value: UInt16(index), index: 2,
                               data: page.toData())
        let result = pollCsDeferred(expectedSlot: CS_LAST_SLOT_DISPLAY_FLAG | UInt8(index))
        fetchCsDisplayPage(index)
        fetchCsStatus()
        DispatchQueue.main.async { self.csDisplayLastStatus = result }
        return result
    }

    /// Arm the IR learn listener (REQ_CS_IR_LEARN wValue=1; spec §3.6.1).  The
    /// device listens on the receiver for up to 10 s for the next cleanly
    /// decoded remote button.  `.noReceiver` if it STALLs (no live IR
    /// component -> CS_STATUS_NO_IR).  USB-only.
    ///
    /// Takes the device lock for the whole learn window, so another session
    /// cannot rewrite the IR setup mid-learn.  It is released by the read
    /// that ends the learn, by cancel, or by `csIrLearnEnd`.
    @discardableResult
    func csIrLearnArm() -> CsIrLearnArmResult {
        if let core = typedCore {   // -- WP SW12 -- CG24: the core holds the device lock for the learn
            // window and ends it on a final read, a cancel, the window's end or
            // the device's going.  A refused lock sends nothing.
            let r = csCoreOp(core, ["op": "learnArm"], write: true)
            if r.ok { return .armed }
            return r.isLocked ? .locked : .noReceiver
        }
        guard transport.acquireDeviceLock(reason: "Learning an IR code", timeout: Self.irLearnLockTimeout) else {
            return .locked
        }
        irLearnLock.held = true
        guard transport.getControlRequest(request: REQ_CS_IR_LEARN, value: CS_IR_LEARN_ARM, index: 2, length: 1) != nil else {
            csIrLearnEnd()
            return .noReceiver
        }
        return .armed
    }

    /// The 10 s learn window plus the UI's polling margin.
    static let irLearnLockTimeout: TimeInterval = 15

    /// Cancel an armed IR learn (REQ_CS_IR_LEARN wValue=0); state returns to idle.
    /// -- WP SW12 -- fixup sw12 N2: Cancel from the main thread, in order.
    /// Through a core the cancel is queued on the core's queue at once, so an
    /// arm issued after it runs after it (it cannot end the new window);
    /// without a core it runs off main as before.
    func csIrLearnCancelInOrder() {
        if let core = typedCore {
            csQueuedWrite(core, lane: nil, ["op": "learnCancel"])
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { self.csIrLearnCancel() }
    }

    func csIrLearnCancel() {
        if let core = typedCore { csCoreOp(core, ["op": "learnCancel"], write: true); return }   // -- WP SW12 -- ends the core's window
        defer { csIrLearnEnd() }
        _ = transport.getControlRequest(request: REQ_CS_IR_LEARN, value: CS_IR_LEARN_CANCEL, index: 2, length: 1)
    }

    /// Read the current IR learn result (REQ_CS_IR_LEARN wValue=2 -> 8 bytes;
    /// spec §3.6.1): the state and, when done, the captured protocol + code.
    /// USB-only; safe to poll while armed.  A result that is no longer armed
    /// ends the learn and releases its lock.
    func csIrLearnRead() -> CsIrLearnResult? {
        if let core = typedCore {   // -- WP SW12 -- a result no longer armed ends the core's window
            let r = csCoreOp(core, ["op": "learnRead"], write: false)
            guard r.ok else { return nil }
            return CsIrLearnResult(state: (r.fields["state"] as? NSNumber)?.uint8Value ?? 0,
                                   proto: (r.fields["protocol"] as? NSNumber)?.uint8Value ?? 0,
                                   code: (r.fields["code"] as? NSNumber)?.uint32Value ?? 0)
        }
        guard let d = transport.getControlRequest(request: REQ_CS_IR_LEARN, value: CS_IR_LEARN_READ, index: 2, length: 8) else { return nil }
        let result = CsIrLearnResult.fromData(d)
        if let r = result, r.state != CS_IR_LEARN_STATE_ARMED { csIrLearnEnd() }
        return result
    }

    /// Release the learn's lock without a device round-trip (the window
    /// closed, the device switched, or polling gave up).  Idempotent, so it
    /// never drops a lock a later flow of this session took.
    func csIrLearnEnd() {
        // -- WP SW12 -- Through a core the arm sets no flag here: the core's
        // learn window ends on its own exits (CG24), so this returns at once.
        guard irLearnLock.take() else { return }
        transport.releaseDeviceLock()
    }

    // MARK: - Firmware install

    /// Restart the device into the bootloader for an install, holding the
    /// device lock so no other session writes to a device about to vanish.
    /// The lock normally dies with the hub's router when the device drops off
    /// the bus; `releaseFirmwareInstallLock` covers a restart that never
    /// happened.  `completion` (main thread) gets false when the lock was
    /// refused and nothing was sent.
    func enterBootloaderForInstall(completion: @escaping (Bool) -> Void) {
        let transport = self.transport
        DispatchQueue.global(qos: .userInitiated).async {
            guard transport.acquireDeviceLock(reason: "Installing firmware", timeout: DeviceLock.maxTimeout) else {
                DispatchQueue.main.async { completion(false) }
                return
            }
            self.firmwareInstallLock.held = true
            // The device drops off the bus answering, so there is no reply.
            _ = transport.getControlRequest(request: REQ_ENTER_BOOTLOADER, value: 0, index: 2, length: 1)
            DispatchQueue.main.async { completion(true) }
        }
    }

    /// Called when the install ends, however it ends.  Idempotent.
    func releaseFirmwareInstallLock() {
        guard firmwareInstallLock.take() else { return }
        transport.releaseDeviceLock()
    }

    // MARK: - Signal Generator

    /// Enumerate siggen capabilities + live state at connect.  The caps
    /// header (REQ_SIGGEN_GET_CAPS, wValue=0xFFFF) doubles as the feature
    /// probe: older firmware STALLs it and the Tools window shows its
    /// unsupported notice.  On success we cache every per-type descriptor
    /// (the authoritative parameter ranges/defaults, spec §3.4) plus the
    /// applied config and status, and seed the editing draft.
    func fetchSiggen() {
        guard let h = transport.getControlRequest(request: REQ_SIGGEN_GET_CAPS, value: SIGGEN_CAPS_HEADER, index: 2, length: 8),
              let caps = SiggenCapsHeader.fromData(h) else {
            DispatchQueue.main.async { self.siggenSupported = false }
            return
        }
        var descs: [SiggenTypeDesc] = []
        for t in 0..<Int(caps.typeCount) {
            guard let d = transport.getControlRequest(request: REQ_SIGGEN_GET_CAPS, value: UInt16(t), index: 2, length: 62),
                  let desc = SiggenTypeDesc.fromData(d) else { continue }
            descs.append(desc)
        }
        // The applied config: a zeroed struct (channelMask == 0) means
        // nothing was ever staged - keep the local draft in that case.
        let applied = transport.getControlRequest(request: REQ_SIGGEN_GET_CONFIG, value: 0, index: 2, length: 36)
            .flatMap(SiggenConfig.fromData)
        DispatchQueue.main.async {
            self.siggenSupported = true
            self.siggenCaps = caps
            self.siggenTypeDescs = descs
            if let cfg = applied, cfg.channelMask != 0 {
                self.siggenDraft = cfg
            } else {
                // Clamp the default draft mask to this platform's outputs.
                self.siggenDraft.channelMask &= caps.validChannelMask
                if self.siggenDraft.channelMask == 0 { self.siggenDraft.channelMask = 1 }
            }
        }
        fetchSiggenStatus()
    }

    /// Read the 16-byte live status (state, elapsed, cycles, sweep freq)
    /// into `siggenStatus`.  Cheap enough to poll while running.
    func fetchSiggenStatus() {
        guard let d = transport.getControlRequest(request: REQ_SIGGEN_GET_STATUS, value: 0, index: 2, length: 16),
              let st = SiggenStatus.fromData(d) else { return }
        DispatchQueue.main.async { self.siggenStatus = st }
    }

    /// Stage a config (OUT, 36 bytes).  Never auto-starts; if the generator
    /// is already running the firmware restarts it with a fade
    /// (stop_reason = RECONFIG), which is what makes live editing safe.
    func siggenSetConfig(_ cfg: SiggenConfig) {
        transport.sendControlRequest(request: REQ_SIGGEN_SET_CONFIG, value: 0, index: 2, data: cfg.toData())
    }

    /// Issue a parameterless transport action (write-as-read: an IN transfer
    /// carrying the action in wValue, acknowledged with one status byte).
    /// Returns false on STALL (unknown action, or START with no staged
    /// config).  Blocks; call off the main thread.
    @discardableResult
    func siggenControl(_ action: UInt16) -> Bool {
        guard let d = transport.getControlRequest(request: REQ_SIGGEN_CONTROL, value: action, index: 2, length: 1),
              d.first == 1 else { return false }
        return true
    }

    /// Stage `cfg` and start (or restart) the generator, then refresh the
    /// status.  The send is queued ahead of the control read on the USB
    /// serial queue, so ordering is guaranteed.  Blocks; call off main.
    func siggenStart(with cfg: SiggenConfig) {
        siggenSetConfig(cfg)
        siggenControl(SIGGEN_CTL_START)
        fetchSiggenStatus()
    }

    /// Stop the generator (faded by default, hard stop when `immediate`),
    /// then refresh the status.  Blocks; call off main.
    func siggenStop(immediate: Bool = false) {
        siggenControl(immediate ? SIGGEN_CTL_STOP_NOW : SIGGEN_CTL_STOP)
        fetchSiggenStatus()
    }

    /// Play the channel-ID ident tone on a single output (matrix output index
    /// 0..8) so a listener can physically locate it.  Stages a CHANNEL_ID
    /// config masked to just that output and starts it: the firmware's walk
    /// position latches to the lowest set mask bit, so it plays exactly
    /// (index + 1) pentatonic blips at that channel's pitch (spec §2.2).  Two
    /// passes then auto-complete to idle - brief but unmistakable.
    ///
    /// Transient, never persisted; leaves the Signal Generator editing draft
    /// untouched (a running user signal is restarted as the ident and resumes
    /// only if the user restarts it).  No-op when the firmware has no
    /// generator.  Dispatches the blocking USB work off the caller's thread,
    /// so it is safe to invoke directly from a menu action on the main thread.
    func identifyOutput(_ outputIndex: Int) {
        guard siggenSupported, outputIndex >= 0, outputIndex < 16 else { return }
        let bit = UInt16(1) << UInt16(outputIndex)
        // Clamp to the platform's valid outputs (caps mask is authoritative);
        // fall back to the raw bit before the first caps fetch.
        let validMask = siggenCaps.validChannelMask != 0 ? siggenCaps.validChannelMask : bit
        let mask = bit & validMask
        guard mask != 0 else { return }
        let cfg = SiggenConfig(
            signalType: SIGGEN_CHANNEL_ID,
            channelMask: mask,
            levelDB: -12.0,        // spec §13.4 ident level; trim/volume still apply
            repeatCount: 2,        // two passes over the single channel, then idle
            p1: 120.0)             // 120 ms blips (CHANNEL_ID default)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.siggenStart(with: cfg)
        }
    }

    // MARK: - Bulk Parameter Transfer

    /// Fetches all DSP parameters in a single V16 (5864-byte) USB transfer.
    /// Parses the flat WireBulkParams structure (unified channel model: inputs
    /// 0..N-1, outputs follow) and updates all published properties.  Returns
    /// true on a valid V16 payload, false otherwise.
    ///
    /// -- WP SW8 -- A device driven through the Rust core is read by the
    /// core instead (`fetchAllParamsThroughCore`, Core/CoreMirror.swift),
    /// which also shows an older firmware's image read-only.  The decoder
    /// below serves the transports not yet on the core (USB without
    /// `-CoreTransport`, MIDI, the Link client) and goes with them.
    @discardableResult
    func fetchAllParams(markDisconnectedOnFailure: Bool = true) -> Bool {
        if let core = transport.coreStateSource { return fetchAllParamsThroughCore(core) }
        let result = transport.getControlResult(request: REQ_GET_ALL_PARAMS, value: 0, index: 2, length: BULK_PARAMS_SIZE)
        guard case .success(let data) = result else {
            // LOCKED means another session is mid-flow on a live device, not a
            // dead link; the notifications from its writes resync us after.
            if markDisconnectedOnFailure, result != .failure(.locked) {
                self.transport.markDisconnected()
            }
            return false
        }
        // The bulk image is all-or-nothing: only the full, current layout is
        // accepted.  A short or wrong-version payload means incompatible
        // firmware - the device is still connected, so don't disconnect (avoids
        // a reconnect loop); just record the version so the UI can react.
        // Requiring the full V32 size keeps the last section (limiter, offset
        // 6028..6135) in range along with every section before it.
        guard data.count >= Int(BULK_PARAMS_SIZE), Int(data[0]) == WIRE_FORMAT_VERSION else {
            DispatchQueue.main.async { self.firmwareWireFormatVersion = Int(data.first ?? 0) }
            return false
        }

        // --- Header (offset 0, 16 bytes) ---
        let formatVersion = data[0]
        let platformId = data[1]
        let numCh = min(Int(data[2]), WIRE_MAX_CHANNELS)          // total channels (7 / 17)
        let numOutCh = min(Int(data[3]), 9)                       // output channels (5 / 9)
        let numInCh = max(min(Int(data[4]), MAX_MATRIX_INPUTS), BASE_MATRIX_INPUTS)  // input channels (2 / 8)
        let platform: String
        switch platformId {
        case 1:  platform = "RP2350"
        case 2:  platform = "STM32H723"
        default: platform = "RP2040"
        }

        // --- Global (offset 16) ---
        let preamp: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_GLOBAL_OFFSET, as: Float.self) }
        let bypassVal = data[BULK_GLOBAL_OFFSET + 4] != 0
        let loudnessEn = data[BULK_GLOBAL_OFFSET + 5] != 0
        let loudnessMask = UInt16(data[BULK_GLOBAL_OFFSET + 6]) | (UInt16(data[BULK_GLOBAL_OFFSET + 7]) << 8)
        let loudnessRef: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_GLOBAL_OFFSET + 8, as: Float.self) }
        let loudnessInt: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_GLOBAL_OFFSET + 12, as: Float.self) }

        // --- Crossfeed (offset 32) ---
        let cfEnabled = data[BULK_CROSSFEED_OFFSET] != 0
        let cfPreset = Int(data[BULK_CROSSFEED_OFFSET + 1])
        let cfITD = data[BULK_CROSSFEED_OFFSET + 2] != 0
        // Output-pair mask lives in the former reserved byte at offset 3 (V20+).
        let cfOutputMask = data[BULK_CROSSFEED_OFFSET + 3]
        let cfFreq: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_CROSSFEED_OFFSET + 4, as: Float.self) }
        let cfFeed: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_CROSSFEED_OFFSET + 8, as: Float.self) }
        // Pair map at offset 12 (extension channel firmware; zero = default on stock).
        let cfPairMap = CrossfeedPairMap(bulk: data)

        // --- Delays (offset 64, float[17]) ---
        var delays = [Int: Float]()
        for i in 0..<numCh {
            delays[i] = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_DELAYS_OFFSET + i * 4, as: Float.self) }
        }

        // --- Matrix crosspoints (offset 132, crosspoints[8][9] inline) ---
        // Backing arrays are always MAX_MATRIX_INPUTS rows; rows beyond numInCh
        // stay disabled / 0 dB on stereo firmware.
        var routing = Array(repeating: Array(repeating: false, count: 9), count: MAX_MATRIX_INPUTS)
        var mxGain = Array(repeating: Array(repeating: Float(0.0), count: 9), count: MAX_MATRIX_INPUTS)
        var mxInvert = Array(repeating: Array(repeating: false, count: 9), count: MAX_MATRIX_INPUTS)
        for input in 0..<numInCh {
            for output in 0..<numOutCh {
                let base = BULK_CROSSPOINT_OFFSET + (input * 9 + output) * WIRE_CROSSPOINT_SIZE
                routing[input][output] = data[base] != 0
                mxInvert[input][output] = data[base + 1] != 0
                mxGain[input][output] = data.withUnsafeBytes { $0.load(fromByteOffset: base + 4, as: Float.self) }
            }
        }

        // --- Matrix outputs (offset 708, WireOutputChannel[9]) ---
        var outEnabled = Array(repeating: false, count: 9)
        var outMuted = Array(repeating: false, count: 9)
        var outGain = Array(repeating: Float(0.0), count: 9)
        var outDelay = Array(repeating: Float(0.0), count: 9)
        for output in 0..<numOutCh {
            let base = BULK_OUTPUTS_OFFSET + output * 12
            outEnabled[output] = data[base] != 0
            outMuted[output] = data[base + 1] != 0
            outGain[output] = data.withUnsafeBytes { $0.load(fromByteOffset: base + 4, as: Float.self) }
            outDelay[output] = data.withUnsafeBytes { $0.load(fromByteOffset: base + 8, as: Float.self) }
        }

        // --- Pin config (offset 816) ---
        let numPins = min(Int(data[BULK_PINS_OFFSET]), 5)
        var pins: [UInt8] = [6, 7, 8, 9, 10]  // defaults
        for i in 0..<numPins {
            pins[i] = data[BULK_PINS_OFFSET + 1 + i]
        }

        // --- EQ bands (offset 824, eq[17][12]) ---
        // WireBandParams layout: type(1) bypass(1) reserved(2) freq(4) q(4) gain_db(4)
        let bandsInFirmware = 12
        let bandsInApp = 10
        var channelFilters = [Int: [FilterParams]]()
        for ch in 0..<numCh {
            var bands = [FilterParams]()
            bands.reserveCapacity(bandsInApp)
            for band in 0..<bandsInApp {
                let base = BULK_EQ_OFFSET + (ch * bandsInFirmware + band) * WIRE_BAND_PARAMS_SIZE
                let typeByte = data[base]
                let bypassByte = data[base + 1]
                let freq: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 4, as: Float.self) }
                let q: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 8, as: Float.self) }
                let gain: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 12, as: Float.self) }
                let type = FilterType(rawValue: Int(typeByte)) ?? .flat
                var fp = FilterParams(
                    type: type,
                    freq: freq,
                    q: q,
                    gain: gain,
                    bypass: bypassByte == 1
                )
                // Reserved bytes 2-3 carry qp_x512 for LT bands (V22+), zero
                // otherwise; little-endian u16.
                if type == .linkwitzTransform {
                    let qpRaw: UInt16 = data.withUnsafeBytes { $0.load(fromByteOffset: base + 2, as: UInt16.self) }
                    fp.qp = FilterParams.decodeQp(qpRaw)
                }
                bands.append(fp)
            }
            channelFilters[ch] = bands
        }

        // --- Channel names (offset 4088, names[17][32]) ---
        var names = [String](repeating: "", count: WIRE_MAX_CHANNELS)
        for ch in 0..<numCh {
            let base = BULK_CHANNEL_NAMES_OFFSET + ch * 32
            let end = min(base + 32, data.count)
            let slice = data[base..<end]
            if let nulIdx = slice.firstIndex(of: 0) {
                names[ch] = String(data: data[base..<nulIdx], encoding: .ascii) ?? ""
            } else {
                names[ch] = String(data: slice, encoding: .ascii) ?? ""
            }
        }

        // --- I2S Config (offset 4632) ---
        var slotTypes: [UInt8] = [0, 0, 0, 0]
        for i in 0..<4 { slotTypes[i] = data[BULK_I2S_OFFSET + i] }
        let bckPin = data[BULK_I2S_OFFSET + 4]
        let mckPinVal = data[BULK_I2S_OFFSET + 5]
        let mckEn = data[BULK_I2S_OFFSET + 6] != 0
        let mckMult = data[BULK_I2S_OFFSET + 7] == 1 ? 256 : 128
        // Clock-pin mode + slave pair claim former reserved bytes (+8/+9), wire
        // version unchanged (18).  clock_pin_mode_p1 is +1 encoded (0=absent on
        // firmware without the feature - the feature-detection signal); slave
        // BCK is a plain GPIO (0=absent → keep the live default).
        let clockPinModeP1 = data[BULK_I2S_OFFSET + 8]
        let bckPinSlaveRaw = data[BULK_I2S_OFFSET + 9]

        // --- Volume Leveller (offset 4648) ---
        let lvlEnabled = data[BULK_LEVELLER_OFFSET] != 0
        let lvlSpeed = Int(data[BULK_LEVELLER_OFFSET + 1])
        let lvlLookahead = data[BULK_LEVELLER_OFFSET + 2] != 0
        let lvlAmount: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_LEVELLER_OFFSET + 4, as: Float.self) }
        let lvlMaxGain: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_LEVELLER_OFFSET + 8, as: Float.self) }
        let lvlGateDB: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_LEVELLER_OFFSET + 12, as: Float.self) }
        let lvlDetectorMask = data[BULK_LEVELLER_OFFSET + 16]   // V18: bit k = input channel k feeds detector
        let lvlApplyMask = data[BULK_LEVELLER_OFFSET + 17]      // V18: bit k = gain applied to input channel k

        // --- Per-Input Preamp (offset 4664, preamp_db[8]) ---
        var preampAll = Array(repeating: Float(0.0), count: MAX_MATRIX_INPUTS)
        for ch in 0..<numInCh {
            preampAll[ch] = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PREAMP_OFFSET + ch * 4, as: Float.self) }
        }

        // --- Master Volume (offset 4696) ---
        let masterVol: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_MASTER_VOLUME_OFFSET, as: Float.self) }

        // --- Input Config (offset 4712) ---
        let bulkInputSource = Int(data[BULK_INPUT_CONFIG_OFFSET])
        let bulkSpdifRxPin = data[BULK_INPUT_CONFIG_OFFSET + 1]
        let bulkI2SRxPin = data[BULK_INPUT_CONFIG_OFFSET + 2]   // pair 0
        let bulkI2SInputRateHz = i2sRateEnumToHz(data[BULK_INPUT_CONFIG_OFFSET + 3])
        // V16+ multichannel I2S: count at +4 (0 = absent), pairs 1..3 at +5..+7
        // (0 = unset → keep the live default).
        let bulkI2SInputChannels = Int(data[BULK_INPUT_CONFIG_OFFSET + 4])
        let bulkI2SRxPinsExt: [UInt8] = [data[BULK_INPUT_CONFIG_OFFSET + 5],
                                         data[BULK_INPUT_CONFIG_OFFSET + 6],
                                         data[BULK_INPUT_CONFIG_OFFSET + 7]]
        // Optional S/PDIF inputs 2/3/4: pins at +8/+9/+10 (0 = keep live), enable
        // mask "plus one" at +11 (0 = field absent; otherwise mask = enc-1,
        // bit0 = SPDIF2 .. bit2 = SPDIF4).  The +1 encoding lets old hosts push
        // zeros meaning "absent" rather than "disable them all".  V28 grew the
        // pin array from 2 to 3, which is what shifted this and the fields below.
        let bulkSpdifRxPinsExt: [UInt8] = [data[BULK_INPUT_CONFIG_OFFSET + 8],
                                           data[BULK_INPUT_CONFIG_OFFSET + 9],
                                           data[BULK_INPUT_CONFIG_OFFSET + 10]]
        let bulkSpdifEnabledExtP1 = data[BULK_INPUT_CONFIG_OFFSET + 11]
        // V21+ I2S clock mode at +12 (0=master, 1=slave).
        let bulkI2SClockMode = data[BULK_INPUT_I2S_CLOCK_MODE_OFFSET]
        // V24+ ADAT input at +13/+14/+15 (RP2350).  Pin: 0 = unset (0xFF never on
        // the wire).  Enable / clock mode are +1 encoded (0 = absent/keep live).
        let bulkAdatInputPinRaw = data[BULK_INPUT_ADAT_PIN_OFFSET]
        let bulkAdatInputPin: UInt8 = bulkAdatInputPinRaw == 0 ? ADAT_INPUT_PIN_UNSET : bulkAdatInputPinRaw
        let bulkAdatInputEnabledP1 = data[BULK_INPUT_ADAT_ENABLED_P1_OFFSET]
        let bulkAdatInputClockModeP1 = data[BULK_INPUT_ADAT_CLOCK_MODE_P1_OFFSET]


        // --- LG Sound Sync (offset 4728) — only `enabled` honoured on SET ---
        let bulkLgEnabled = data[BULK_LG_OFFSET] != 0

        // --- User Volume (offset 4744) ---
        let bulkUserVolume: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_USER_VOLUME_OFFSET, as: Float.self) }

        // --- DAC Hardware Mute (offset 4760, 16 bytes) ---
        let bulkDacHwMute = DacHwMuteConfig.fromData(data.subdata(in: BULK_DAC_HW_MUTE_OFFSET..<(BULK_DAC_HW_MUTE_OFFSET + 16))) ?? DacHwMuteConfig()

        // --- Crossover Config (offset 4776, crossovers[17][4]) ---
        // Input rows (ch < chOut1) come back zeroed; stored as FLAT and hidden.
        var bulkCrossovers = [Int: [FilterParams]]()
        for ch in 0..<numCh {
            var bands: [FilterParams] = []
            bands.reserveCapacity(WIRE_MAX_XOVER_BANDS)
            for band in 0..<WIRE_MAX_XOVER_BANDS {
                let base = BULK_CROSSOVER_OFFSET + (ch * WIRE_MAX_XOVER_BANDS + band) * WIRE_BAND_PARAMS_SIZE
                let typeByte = data[base]
                let bypassByte = data[base + 1]
                let freq: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 4, as: Float.self) }
                let q: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 8, as: Float.self) }
                let gain: Float = data.withUnsafeBytes { $0.load(fromByteOffset: base + 12, as: Float.self) }
                bands.append(FilterParams(
                    type: FilterType(rawValue: Int(typeByte)) ?? .flat,
                    freq: max(freq, 10),
                    q: q > 0 ? q : 0.707,
                    gain: gain,
                    bypass: bypassByte == 1
                ))
            }
            bulkCrossovers[ch] = bands
        }

        // --- ADAT bulk output config (offset 5864, WireAdatConfig) ---
        // Live config: enabled + data pin (pin 0 => platform default).  RP2040
        // reports zeros; ADAT support is gated on platform below.
        let bulkAdatEnabled = data[BULK_ADAT_OFFSET] != 0
        let bulkAdatPinRaw = data[BULK_ADAT_OFFSET + 1]
        let bulkAdatPin = bulkAdatPinRaw == 0 ? ADAT_PIN_DEFAULT : bulkAdatPinRaw

        // --- Psychoacoustic Bass (offset 5876, WirePsybassParams 24 bytes) ---
        let pbEnabled = data[BULK_PSYBASS_OFFSET] != 0
        let pbOutputMask = UInt16(data[BULK_PSYBASS_OFFSET + 2]) | (UInt16(data[BULK_PSYBASS_OFFSET + 3]) << 8)
        let pbCutoff: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PSYBASS_OFFSET + 4, as: Float.self) }
        let pbHarmonics: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PSYBASS_OFFSET + 8, as: Float.self) }
        let pbDrive: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PSYBASS_OFFSET + 12, as: Float.self) }
        let pbCharacter: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PSYBASS_OFFSET + 16, as: Float.self) }
        let pbOriginal: Float = data.withUnsafeBytes { $0.load(fromByteOffset: BULK_PSYBASS_OFFSET + 20, as: Float.self) }

        // --- Stereo Upmixer (offset 5900, WireUpmixParams 44 bytes) ---
        let umEnabled = data[BULK_UPMIX_OFFSET] != 0
        let umCenterMode = Int(data[BULK_UPMIX_OFFSET + 1])
        let umSurroundMode = Int(data[BULK_UPMIX_OFFSET + 2])
        let umPresence = Float(Int8(bitPattern: data[BULK_UPMIX_OFFSET + 3])) / 2.0  // presence_q1 -> dB
        func umF(_ off: Int) -> Float { data.withUnsafeBytes { $0.load(fromByteOffset: BULK_UPMIX_OFFSET + off, as: Float.self) } }
        let umStrength = umF(4), umWidth = umF(8), umThreshold = umF(12), umAttack = umF(16), umRelease = umF(20)
        let umDetHpf = umF(24), umSurDelay = umF(28), umSurHpf = umF(32), umSurLpf = umF(36), umDecorr = umF(40)

        // --- Subharmonic Synthesizer (offset 5944, WireSubharmParams 36 bytes) ---
        // `solo` is deliberately not on the wire, so it is not parsed here and a
        // bulk apply can never switch the program signal off.
        let shEnabled = data[BULK_SUBHARM_OFFSET] != 0
        let shOutputMask = UInt16(data[BULK_SUBHARM_OFFSET + 2]) | (UInt16(data[BULK_SUBHARM_OFFSET + 3]) << 8)
        func shF(_ off: Int) -> Float { data.withUnsafeBytes { $0.load(fromByteOffset: BULK_SUBHARM_OFFSET + off, as: Float.self) } }
        let shLow = shF(4), shHigh = shF(8), shBoost = shF(12)
        let shTop = shF(16), shDepth = shF(20), shHold = shF(24), shCeiling = shF(28)
        let shSelectMode = min(Int(data[BULK_SUBHARM_OFFSET + 32]), SUBHARM_SELECT_SUSTAINED)
        let shLinkPairs = data[BULK_SUBHARM_OFFSET + 33] != 0

        // --- Tube Modeller (offset 5980, WireTubeParams 48 bytes) ---
        // Restored verbatim, with no row lookup: a preset saved as a type keeps
        // its stored knobs even if a later firmware retunes that row.
        let tbEnabled = data[BULK_TUBE_OFFSET] != 0
        let tbType = min(Int(data[BULK_TUBE_OFFSET + 1]), TUBE_TYPE_MAX)
        let tbRectifier = min(Int(data[BULK_TUBE_OFFSET + 2]), TUBE_RECT_MAX)
        let tbXfmrEnabled = data[BULK_TUBE_OFFSET + 3] != 0
        let tbOutputMask = UInt16(data[BULK_TUBE_OFFSET + 4]) | (UInt16(data[BULK_TUBE_OFFSET + 5]) << 8)
        func tbF(_ off: Int) -> Float { data.withUnsafeBytes { $0.load(fromByteOffset: BULK_TUBE_OFFSET + off, as: Float.self) } }
        let tbDrive = tbF(8), tbBias = tbF(12), tbAsym = tbF(16), tbHardness = tbF(20), tbSag = tbF(24)
        let tbXfmrDamping = tbF(28), tbXfmrRes = tbF(32), tbMix = tbF(36), tbTrim = tbF(40)

        // --- Output Limiter (offset 6028, WireLimiterParams 108 bytes) ---
        // One record per wire slot; slots past this device's outputs are zero
        // and are read as defaults rather than as a -0 dBFS, 0 ms limiter.
        var lmOutputs = [LimiterOutputSettings](repeating: LimiterOutputSettings(),
                                                count: WIRE_MAX_OUTPUT_CHANNELS)
        for k in 0..<min(numOutCh, WIRE_MAX_OUTPUT_CHANNELS) {
            let o = BULK_LIMITER_OFFSET + k * WIRE_LIMITER_OUTPUT_SIZE
            func lmF(_ off: Int) -> Float { data.withUnsafeBytes { $0.load(fromByteOffset: o + off, as: Float.self) } }
            lmOutputs[k] = LimiterOutputSettings(
                enabled: data[o] != 0,
                thresholdDB: Self.clampLimiterThreshold(lmF(4)),
                releaseMs: Self.clampLimiterRelease(lmF(8)),
                linkGroup: LimiterGang.storedGroup(data[o + 1]))
        }
        // A restore can leave a group disagreeing (an older preset).  The
        // firmware gangs it at its next recompute and notifies each change;
        // doing the same here keeps a read taken before that from showing
        // settings that are about to change.
        LimiterGang.gangAll(&lmOutputs, count: min(numOutCh, WIRE_MAX_OUTPUT_CHANNELS))

        // --- Apply all parsed values on main thread ---
        DispatchQueue.main.async {
            self.platformName = platform
            self.numInputChannels = numInCh
            _ = preamp  // legacy global preamp; per-input preamp is authoritative

            self.preampDB = preampAll
            self.masterVolumeDB = masterVol
            self.bypass = bypassVal
            self.loudnessEnabled = loudnessEn
            self.loudness.outputMask = loudnessMask
            self.loudness.refSPL = loudnessRef
            self.loudness.intensity = loudnessInt

            self.crossfeedEnabled = cfEnabled
            self.crossfeed.preset = cfPreset
            self.crossfeed.itd = cfITD
            self.crossfeed.freq = cfFreq
            self.crossfeed.feed = cfFeed
            self.crossfeed.outputMask = cfOutputMask
            if self.crossfeed.pairMap != cfPairMap { self.crossfeed.pairMap = cfPairMap }

            self.psybassEnabled = pbEnabled
            self.psybass.outputMask = pbOutputMask
            self.psybass.cutoffHz = pbCutoff
            self.psybass.harmonicsDB = pbHarmonics
            self.psybass.driveDB = pbDrive
            self.psybass.characterPct = pbCharacter
            self.psybass.originalDB = pbOriginal

            self.subharmEnabled = shEnabled
            self.subharm.outputMask = shOutputMask
            self.subharm.lowDB = shLow
            self.subharm.highDB = shHigh
            self.subharm.topDB = shTop
            self.subharm.boostDB = shBoost
            self.subharm.selectMode = shSelectMode
            self.subharm.selectDepthPct = shDepth
            self.subharm.selectHoldMs = shHold
            self.subharm.ceilingDB = shCeiling
            self.subharm.linkPairs = shLinkPairs

            self.tubeEnabled = tbEnabled
            self.tube.outputMask = tbOutputMask
            self.tube.type = tbType
            self.tube.driveDB = tbDrive
            self.tube.biasPct = tbBias
            self.tube.asymDB = tbAsym
            self.tube.hardnessPct = tbHardness
            self.tube.sagPct = tbSag
            self.tube.rectifier = tbRectifier
            self.tube.xfmrEnabled = tbXfmrEnabled
            self.tube.xfmrDamping = tbXfmrDamping
            self.tube.xfmrResHz = tbXfmrRes
            self.tube.mixPct = tbMix
            self.tube.trimDB = tbTrim

            self.limiter.outputs = lmOutputs

            self.upmixEnabled = umEnabled
            self.upmix.centerMode = umCenterMode
            self.upmix.surroundMode = umSurroundMode
            self.upmix.presenceDB = umPresence
            self.upmix.strengthPct = umStrength
            self.upmix.centerWidthPct = umWidth
            self.upmix.thresholdPct = umThreshold
            self.upmix.attackMs = umAttack
            self.upmix.releaseMs = umRelease
            self.upmix.detectorHpfHz = umDetHpf
            self.upmix.surroundDelayMs = umSurDelay
            self.upmix.surroundHpfHz = umSurHpf
            self.upmix.surroundLpfHz = umSurLpf
            self.upmix.decorrPct = umDecorr

            self.channelDelays = delays

            self.matrixRouting = routing
            self.matrixGain = mxGain
            self.matrixInvert = mxInvert

            self.outputEnabled = outEnabled
            self.outputMuted = outMuted
            self.outputGainDB = outGain
            self.outputDelayMS = outDelay

            self.outputPins = pins

            self.outputSlotTypes = slotTypes
            self.i2sBckPin = bckPin
            self.mckPin = mckPinVal
            self.mckEnabled = mckEn
            self.mckMultiplier = mckMult
            // Clock-pin mode: a nonzero +1-encoded value is the feature-detection
            // signal (older firmware leaves the reserved byte 0).  Slave BCK 0
            // means "unset"; keep the live default in that case.
            if clockPinModeP1 != 0 {
                self.i2sClockPinModeSupported = true
                self.i2sClockPinMode = clockPinModeP1 - 1
            }
            if bckPinSlaveRaw != 0 { self.i2sBckPinSlave = bckPinSlaveRaw }

            self.levellerEnabled = lvlEnabled
            self.leveller.amount = lvlAmount
            self.leveller.speed = lvlSpeed
            self.leveller.maxGainDB = lvlMaxGain
            self.leveller.lookahead = lvlLookahead
            self.leveller.gateDB = lvlGateDB
            self.leveller.detectorMask = lvlDetectorMask
            self.leveller.applyMask = lvlApplyMask

            self.channelData = channelFilters
            self.channelNames = names

            self.inputSource = bulkInputSource
            self.inputSourceSupported = true
            self.spdifRxPin = bulkSpdifRxPin
            // Optional S/PDIF 2/3/4: a non-zero enable-mask+1 byte marks the
            // multiple-input feature present and carries the live enable state;
            // ext pins of 0 mean "absent, keep the live pin".  The count comes
            // from the 0xEF inventory (fetchSpdifInputConfig), which is the only
            // thing that knows how many inputs this firmware actually has; bulk
            // just fills in the state.  Assume the full count until it lands.
            if bulkSpdifEnabledExtP1 != 0 {
                self.multiSpdifSupported = true
                if self.spdifInputCount < 2 { self.spdifInputCount = SPDIF_RX_NUM_INPUTS }
                let mask = bulkSpdifEnabledExtP1 - 1
                for i in 0..<(SPDIF_RX_NUM_INPUTS - 1) {
                    self.spdifExtEnabled[i] = (mask & (1 << UInt8(i))) != 0
                    if bulkSpdifRxPinsExt[i] != 0 { self.spdifRxPinsExt[i] = bulkSpdifRxPinsExt[i] }
                }
            }
            self.i2sInputSupported = true
            self.i2sRxPins[0] = bulkI2SRxPin
            for pair in 1..<min(4, self.i2sRxPins.count) where bulkI2SRxPinsExt[pair - 1] != 0 {
                self.i2sRxPins[pair] = bulkI2SRxPinsExt[pair - 1]
            }
            if bulkI2SInputChannels != 0 { self.i2sInputChannels = bulkI2SInputChannels }
            self.i2sInputRateHz = bulkI2SInputRateHz
            // Clock mode: the bulk payload is V21 by definition (strict version
            // gate above), so the device supports the clock-slave feature.  The
            // richer live lock state arrives via fetchI2SClockMode/0x09.
            self.i2sClockModeSupported = true
            self.i2sClockMode = bulkI2SClockMode
            self.i2sSlaveStatus.clockMode = bulkI2SClockMode
            self.userVolumeDB = bulkUserVolume
            self.lgSoundSyncEnabled = bulkLgEnabled
            self.lgSoundSyncSupported = true
            self.dacHwMuteConfig = bulkDacHwMute
            self.dacHwMuteSupported = true

            // ADAT bulk output is RP2350-only (the engine is compiled out on
            // RP2040, which reports zeros here).  fetchAdatConfig() refines this
            // with live active/rate status right after the bulk fetch.
            self.adatSupported = (platform == "RP2350")
            self.adatEnabled = bulkAdatEnabled
            self.adatPin = bulkAdatPin
            self.adatStatus.enabled = bulkAdatEnabled
            self.adatStatus.pin = bulkAdatPin

            // ADAT input (V24+): same RP2350 gate as the bulk output.  The +1
            // encoded enable / clock-mode fields decode as absent (keep live) when
            // zero; fetchAdatInputConfig() refines this with the live lock status
            // right after the bulk fetch.
            self.adatInputSupported = (platform == "RP2350")
            self.adatInputPin = bulkAdatInputPin
            self.adatInputStatus.pin = bulkAdatInputPin
            if bulkAdatInputEnabledP1 != 0 {
                let en = bulkAdatInputEnabledP1 == 2
                self.adatInputEnabled = en
                self.adatInputStatus.enabled = en
            }
            if bulkAdatInputClockModeP1 != 0 {
                let mode = bulkAdatInputClockModeP1 == 2 ? ADAT_INPUT_CLOCK_MODE_SLAVE : ADAT_INPUT_CLOCK_MODE_MASTER
                self.adatInputClockMode = mode
                self.adatInputStatus.clockMode = mode
            }

            self.firmwareWireFormatVersion = Int(formatVersion)

            // V16 always carries crossover state (input rows zeroed); types use
            // the V13+ 32..63 numbering that matches FilterType.
            self.firmwareSupportsCrossover = true
            self.xoverData = bulkCrossovers

            // Refresh channel visibility now that platform/outputs are populated.
            // (chOut1 reflects the freshly-set platformName.)  Any dashboard
            // snapshot predates this channel set, so drop it - leaving a channel
            // page now re-derives the defaults instead of restoring stale pills.
            self.savedOverviewVisibility = nil
            if self.isOverviewMode {
                for i in 0..<self.chOut1 {
                    self.channelVisibility[i] = (i < self.effectiveInputChannels)
                }
                for outputIdx in 0..<numOutCh {
                    self.channelVisibility[self.eqChannel(forOutput: outputIdx)] = outEnabled[outputIdx]
                }
            }

            self.recomputeAllMagnitudes()
            self.updateSavedSnapshot()
        }

        return true
    }

    // MARK: - Preset Commands

    @discardableResult
    func fetchPresetDirectory() -> UInt16 {
        guard let data = transport.getControlRequest(request: REQ_PRESET_GET_DIR, value: 0, index: 2, length: 7),
              data.count >= 6 else { return 0 }
        let occupied = data.withUnsafeBytes { $0.load(as: UInt16.self) }
        let startupMode = Int(data[2])
        let defaultSlot = Int(data[3])
        let lastActive = data[4]
        // Byte [5] carries output_config_mode (0 = independent, 1 = with preset,
        // the repurposed former include_pins flag).  Anything other than 1
        // (including legacy 0) reads as independent.
        let outputConfigMode: Int = (data[5] == UInt8(OUTPUT_CONFIG_MODE_WITH_PRESET))
            ? OUTPUT_CONFIG_MODE_WITH_PRESET
            : OUTPUT_CONFIG_MODE_INDEPENDENT
        // Byte [6] carries master_volume_mode (0 = independent, 1 = with preset).
        // Older firmware that pre-dates the byte returns < 7 bytes; default to mode 0.
        let masterVolMode: Int = (data.count >= 7 && data[6] == UInt8(MASTER_VOLUME_MODE_WITH_PRESET))
            ? MASTER_VOLUME_MODE_WITH_PRESET
            : MASTER_VOLUME_MODE_INDEPENDENT
        DispatchQueue.main.async {
            self.presetOccupied = occupied
            self.presetStartupMode = startupMode
            self.presetDefaultSlot = defaultSlot
            self.activePresetSlot = Int(lastActive)
            self.presetOutputConfigMode = outputConfigMode
            self.presetMasterVolumeMode = masterVolMode
            // Ensure UI cannot show stale names for slots that are not occupied.
            for slot in 0..<10 where (occupied & UInt16(1 << slot)) == 0 {
                self.presetNames[slot] = ""
            }
        }
        return occupied
    }

    func fetchPresetName(slot: Int) {
        guard let data = transport.getControlRequest(request: REQ_PRESET_GET_NAME, value: UInt16(slot), index: 2, length: 32) else {
            DispatchQueue.main.async {
                self.presetNames[slot] = ""
            }
            return
        }
        let name: String
        if let nulIndex = data.firstIndex(of: 0) {
            name = String(data: data[0..<nulIndex], encoding: .ascii) ?? ""
        } else {
            name = String(data: data, encoding: .ascii) ?? ""
        }
        DispatchQueue.main.async {
            self.presetNames[slot] = name
        }
    }

    func fetchPresetActive() {
        guard let data = transport.getControlRequest(request: REQ_PRESET_GET_ACTIVE, value: 0, index: 2, length: 1) else { return }
        DispatchQueue.main.async {
            self.activePresetSlot = Int(data[0])
        }
    }

    /// Save live state to `slot` under the device lock (spec 7.7), so no other
    /// session's write lands in the snapshot.  PRESET_STATUS_LOCKED when
    /// another session holds the lock; nothing is sent then.
    @discardableResult
    func savePreset(slot: Int) -> UInt8 {
        transport.withDeviceLock(reason: "Saving preset", timeout: 5) {
            savePresetUnlocked(slot: slot)
        } ?? PRESET_STATUS_LOCKED
    }

    /// The save itself, for callers that already hold the lock.  Taking it
    /// again here would release it early, since a release is not counted.
    private func savePresetUnlocked(slot: Int) -> UInt8 {
        guard let data = transport.getControlRequest(request: REQ_PRESET_SAVE, value: UInt16(slot), index: 2, length: 1) else { return 0xFF }
        let status = data[0]
        if status == PRESET_OK {
            // -- WP SWF6 -- Through the core, its baseline is taken here, off
            // main and right after the save.
            let source = transport.coreStateSource
            let changes = source?.capturePresetBaseline()
            DispatchQueue.main.async {
                self.activePresetSlot = slot
                self.presetOccupied |= UInt16(1 << slot)
                if let changes = changes, self.unsavedChangesFromCore {
                    self.coreMirror.showPresetChanges(changes, from: source)
                } else {
                    self.updateSavedSnapshot()
                }
            }
        }
        return status
    }

    /// Copy the current live DSP state into `destinationSlot` while leaving
    /// `sourceSlot` as the active preset.
    ///
    /// Mechanism:
    ///   1. Save live → destination.  Wait for the deferred firmware save to
    ///      finish (signaled by REQ_PRESET_GET_ACTIVE returning destinationSlot).
    ///   2. Re-save live → source.  Wait for that save to finish.
    ///
    /// The wait between the two saves is essential: REQ_PRESET_SAVE is
    /// deferred on the firmware (the USB handler queues a single pending
    /// slot and returns PRESET_OK immediately, with the actual flash work
    /// happening in the main loop).  Without the wait, the second save
    /// overwrites `pending_preset_save_slot` before the first one has been
    /// processed, so the first slot is never written — the user sees the
    /// destination slot getting a name (from setPresetName, which writes
    /// the directory directly) but factory-default parameters when loaded.
    ///
    /// One lock covers both saves and both waits, so no other session can
    /// write between the destination and source snapshots.
    ///
    /// Returns PRESET_OK on success, an error code on failure.
    @discardableResult
    func copyPreset(from sourceSlot: Int, to destinationSlot: Int) -> UInt8 {
        transport.withDeviceLock(reason: "Copying preset", timeout: 10) {
            copyPresetUnlocked(from: sourceSlot, to: destinationSlot)
        } ?? PRESET_STATUS_LOCKED
    }

    private func copyPresetUnlocked(from sourceSlot: Int, to destinationSlot: Int) -> UInt8 {
        let generation = transport.generation
        let dstStatus = savePresetUnlocked(slot: destinationSlot)
        guard dstStatus == PRESET_OK else { return dstStatus }
        guard waitForPresetActivation(slot: destinationSlot) else { return 0xFF }

        // A device switch during the deferred wait must not let the source
        // re-save (which also moves last_active) run against the new device.
        guard transport.generation == generation else { return 0xFF }

        let srcStatus = savePresetUnlocked(slot: sourceSlot)
        guard srcStatus == PRESET_OK else { return srcStatus }
        _ = waitForPresetActivation(slot: sourceSlot)

        return PRESET_OK
    }

    @discardableResult
    func loadPreset(slot: Int) -> UInt8 {
        print("[PRESET] loadPreset(\(slot)) starting")
        // -- WP SWF6 -- Through the core: its load re-reads and rebases, and an
        // edit made before its notifications arrive stays unsaved (CG33).
        if let source = transport.coreStateSource {
            let status = loadPresetThroughCore(source, slot: slot)
            print("[PRESET] loadPreset(\(slot)) through the core, status=\(status)")
            return status
        }
        // We refetch everything below, so drop the PRESET_LOADED /
        // BULK_INVALIDATED pair the device is about to send rather than pay for
        // a second bulk read of the state we are already fetching.  Armed
        // before the request so the notification can never beat it.
        suppressDeviceResync()
        guard let status = transport.getControlRequest(request: REQ_PRESET_LOAD, value: UInt16(slot), index: 2, length: 1)?.first else {
            // nil = USB request failed; empty Data = device returned no status
            // byte (e.g. it reset/disconnected mid-transfer).  Treat both as a
            // failure rather than indexing [0] on an empty buffer (which traps).
            print("[PRESET] loadPreset(\(slot)) USB request failed (nil/empty)")
            return 0xFF
        }
        print("[PRESET] loadPreset(\(slot)) device status=\(status)")
        if status == PRESET_OK {
            // Publish the new active slot BEFORE the bulk refresh so the UI
            // converges quickly and the NSPopUpButton's native click-set
            // selection state reconciles with @Binding's view of truth.
            //
            // Why this matters: when the user picks a slot from the preset
            // popup, NSPopUpButton immediately sets .state=.on on the clicked
            // NSMenuItem (native click handling).  If activePresetSlot never
            // follows, the next BorderlessPopUpButton.updateNSView call
            // (triggered by any @Published poll update) will call
            // selectItem(withTag:) with the STALE activePresetSlot — which,
            // depending on NSPopUpButton's internal selectedItem-pointer
            // state vs. the NSMenuItem .state fields, can leave a checkmark
            // on BOTH the old and new slots until something else resyncs.
            // Mirroring savePreset's behavior (Commands.swift:~1144) keeps
            // the invariant "on PRESET_OK, activePresetSlot reflects the
            // slot the device is now on" uniform across both ops.
            DispatchQueue.main.async {
                self.activePresetSlot = slot
            }
            Thread.sleep(forTimeInterval: 0.1)
            fetchAllParams()
            // Rebase the unsaved-changes baseline to the just-loaded slot.
            // Without this, savedSnapshot still reflects whatever slot was
            // active before this load, and hasUnsavedChanges will spuriously
            // report differences (firing the "save changes?" dialog on the
            // very next preset switch).  Queued AFTER fetchAllParams so that
            // all of fetchAllParams's main.async writes have settled by the
            // time captureSnapshot() runs — FIFO ordering on the main queue
            // guarantees this.
            DispatchQueue.main.async {
                self.updateSavedSnapshot()
            }
        }
        return status
    }

    private func waitForPresetActivation(slot: Int, timeout: TimeInterval = 1.5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let data = transport.getControlRequest(request: REQ_PRESET_GET_ACTIVE, value: 0, index: 2, length: 1),
               data.count >= 1,
               Int(data[0]) == slot {
                return true
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        return false
    }

    @discardableResult
    func deletePreset(slot: Int) -> UInt8 {
        guard let data = transport.getControlRequest(request: REQ_PRESET_DELETE, value: UInt16(slot), index: 2, length: 1) else { return 0xFF }
        let status = data[0]
        if status == PRESET_OK {
            DispatchQueue.main.async {
                self.presetOccupied &= ~UInt16(1 << slot)
            }
        }
        return status
    }

    /// Poll REQ_PRESET_GET_DIR until the firmware has actually finished
    /// processing a deferred delete for `slot` (slot_occupied bit cleared).
    ///
    /// Like preset save, REQ_PRESET_DELETE is deferred — the USB handler sets
    /// `preset_delete_mask |= bit(slot)` and returns PRESET_OK immediately,
    /// while the actual flash erase + directory flush happens in the main
    /// loop.  Callers that need to read post-delete directory state (e.g.,
    /// to refresh `presetOccupied` / `presetNames`) must wait for the bit
    /// to clear, otherwise they'll race the deferred work and overwrite
    /// their optimistic local state with the firmware's stale value.
    ///
    /// Returns true on observed completion, false on timeout (default 1.5 s
    /// of polling at 20 ms intervals).
    @discardableResult
    func waitForPresetDeletion(slot: Int, timeout: TimeInterval = 1.5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let data = transport.getControlRequest(request: REQ_PRESET_GET_DIR, value: 0, index: 2, length: 7),
               data.count >= 2 {
                let occupied = UInt16(data[0]) | (UInt16(data[1]) << 8)
                if (occupied & UInt16(1 << slot)) == 0 {
                    return true
                }
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        return false
    }

    func setPresetName(slot: Int, name: String) {
        var nameData = Data(name.prefix(31).utf8)
        nameData.append(0) // NUL terminator
        transport.sendControlRequest(request: REQ_PRESET_SET_NAME, value: UInt16(slot), index: 2, data: nameData)
        DispatchQueue.main.async {
            self.presetNames[slot] = String(name.prefix(31))
        }
    }

    func setPresetStartup(mode: Int, defaultSlot: Int) {
        let data = Data([UInt8(mode), UInt8(defaultSlot)])
        DispatchQueue.main.async {
            self.presetStartupMode = mode
            self.presetDefaultSlot = defaultSlot
        }
        transport.sendControlRequest(request: REQ_PRESET_SET_STARTUP, value: 0, index: 2, data: data)
    }

    func fetchPresetStartup() {
        guard let data = transport.getControlRequest(request: REQ_PRESET_GET_STARTUP, value: 0, index: 2, length: 3),
              data.count >= 3 else { return }
        DispatchQueue.main.async {
            self.presetStartupMode = Int(data[0])
            self.presetDefaultSlot = Int(data[1])
            self.activePresetSlot = Int(data[2])
        }
    }

    // MARK: - Output Config Mode (preset directory flag)

    /// Set output-config persistence mode (the repurposed include-pins flag).
    /// Pass `OUTPUT_CONFIG_MODE_WITH_PRESET` (1, default) to make the whole IO
    /// block — pins, output types, I2S MCK/BCK, and the S/PDIF RX pin — part of
    /// each preset, or `OUTPUT_CONFIG_MODE_INDEPENDENT` (0) for device-global
    /// wiring that's stored in the directory and applied at boot.
    func setOutputConfigMode(_ mode: Int) {
        let clamped = UInt8(max(0, min(1, mode)))
        let normalized = (Int(clamped) == OUTPUT_CONFIG_MODE_WITH_PRESET) ? OUTPUT_CONFIG_MODE_WITH_PRESET
                                                                          : OUTPUT_CONFIG_MODE_INDEPENDENT
        DispatchQueue.main.async { self.presetOutputConfigMode = normalized }
        transport.sendControlRequest(request: REQ_SET_OUTPUT_CONFIG_MODE, value: 0, index: 2, data: Data([clamped]))
    }

    func fetchOutputConfigMode() {
        guard let data = transport.getControlRequest(request: REQ_GET_OUTPUT_CONFIG_MODE, value: 0, index: 2, length: 1) else { return }
        let val = Int(data[0])
        DispatchQueue.main.async {
            self.presetOutputConfigMode = (val == OUTPUT_CONFIG_MODE_WITH_PRESET) ? OUTPUT_CONFIG_MODE_WITH_PRESET
                                                                                  : OUTPUT_CONFIG_MODE_INDEPENDENT
        }
    }

    /// Persist the current live output configuration into the directory's
    /// independent storage so it survives a reboot. Relevant in
    /// `OUTPUT_CONFIG_MODE_INDEPENDENT`, where per-field edits (pins, output
    /// types, I2S clocks, S/PDIF RX pin, output limiters) apply live but only
    /// persist after an explicit save. IN-shaped action command: device responds with a 1-byte
    /// status (0 = PRESET_OK); the flash write is deferred on-device. Returns
    /// true on success, false if the device disconnected or the transfer failed.
    @discardableResult
    func saveOutputConfig() -> Bool {
        if case .success = saveOutputConfigResult() { return true }
        return false
    }

    /// As `saveOutputConfig`, but tells a held device lock (`.locked`, nothing
    /// sent) apart from a device that did not acknowledge (`.error`).
    func saveOutputConfigResult() -> Result<Void, LinkStatus> {
        transport.withDeviceLock(reason: "Saving output configuration", timeout: 5) {
            guard let data = transport.getControlRequest(request: REQ_SAVE_OUTPUT_CONFIG, value: 0, index: 2, length: 1),
                  data.first == 0 else {  // PRESET_OK
                return .failure(.error)
            }
            return .success(())
        } ?? .failure(.locked)
    }

    // MARK: - Channel Names

    func setChannelName(channel: Int, name: String) {
        DispatchQueue.main.async {
            self.channelNames[channel] = String(name.prefix(31))
        }
        // -- WP SW11 -- Through a core, its setter (31 bytes, cut on a character).
        if typedWrite({ try $0.setChannelName(ch: Self.wire(channel), name: String(name.prefix(31))) }) { return }
        var nameData = Data(count: 32)  // Always send full 32-byte buffer, zero-padded
        let utf8 = Data(name.prefix(31).utf8)
        nameData.replaceSubrange(0..<utf8.count, with: utf8)
        transport.sendControlRequest(request: REQ_SET_CHANNEL_NAME, value: UInt16(channel), index: 2, data: nameData)
    }

    func fetchChannelName(channel: Int) {
        if let core = typedCore {   // -- WP SW11 -- its re-read, into the mirror shown
            _ = core.readAndPublish { _ = try $0.getChannelName(ch: Self.wire(channel)) }
            return
        }
        guard let data = transport.getControlRequest(request: REQ_GET_CHANNEL_NAME, value: UInt16(channel), index: 2, length: 32) else { return }
        let name: String
        if let nulIndex = data.firstIndex(of: 0) {
            name = String(data: data[0..<nulIndex], encoding: .ascii) ?? ""
        } else {
            name = String(data: data, encoding: .ascii) ?? ""
        }
        DispatchQueue.main.async {
            self.channelNames[channel] = name
        }
    }

    // MARK: - Notification → State Mirroring

    /// Apply a v2 PARAM_CHANGED notification (already filtered to non-HOST
    /// sources) to local UI state.  Runs on the main thread.
    ///
    /// -- WP SW8 -- Not used for a device driven through the core: there the
    /// core's mirror follows every notification and `CoreMirror` applies
    /// what changed (Core/CoreMirror.swift).
    ///
    /// The listed offsets are decoded in place: EQ and crossover bands, channel
    /// names, dac_hw_mute, user volume, LG, and the input / I2S / ADAT config.
    /// Anything else - the DSP feature blocks (loudness, crossfeed, leveller,
    /// psybass, subharm, tube, limiter, upmixer, preamp, output gain / mute / delay) - falls through to
    /// a coalesced full re-read instead, which costs one bulk transfer but
    /// needs no per-block decoder here and cannot drift out of step with the
    /// wire format.  That path is what lets a bound pot or IR button move those
    /// parameters and have the matching window follow (spec §7.3,
    /// PARAM_SRC_GPIO).  Note InterruptMonitor.decode names every offset, but
    /// that is the diagnostics log only - naming an offset there mirrors
    /// nothing.
    func applyNotifiedParamChange(offset: UInt16, size: UInt16, payload: Data) {
        let off = Int(offset)
        let sz = Int(size)

        // EQ band updates: 17 channels × 12 bands × 16 bytes at the V16 EQ
        // offset (824).  Firmware sends a full WireBandParams (16 bytes) on any
        // band change, including REQ_SET_BAND_BYPASS — see band_bypass_spec §6.4.
        // Decode and update channelData so other hosts' edits flow into our
        // UI without a re-poll.
        let eqBase = BULK_EQ_OFFSET
        let bandsInFirmware = 12
        let bandSize = WIRE_BAND_PARAMS_SIZE
        let channelCount = WIRE_MAX_CHANNELS
        if sz == bandSize,
           off >= eqBase,
           off < eqBase + channelCount * bandsInFirmware * bandSize,
           (off - eqBase) % bandSize == 0,
           payload.count >= bandSize {
            let flatIdx = (off - eqBase) / bandSize
            let ch = flatIdx / bandsInFirmware
            let band = flatIdx % bandsInFirmware
            // App tracks 10 bands per channel; firmware tracks 12 — ignore
            // updates for the two bands we don't surface.
            if band < 10, var bands = self.channelData[ch], band < bands.count {
                let typeByte = payload[0]
                let bypassByte = payload[1]
                let freq: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 4, as: Float.self) }
                let q: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 8, as: Float.self) }
                let gain: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 12, as: Float.self) }
                let type = FilterType(rawValue: Int(typeByte)) ?? .flat
                var fp = bands[band]
                fp.type = type
                fp.freq = freq
                fp.q = q
                fp.gain = gain
                fp.bypass = bypassByte == 1
                // Reserved bytes 2-3 carry qp_x512 for LT bands (V22+).
                if type == .linkwitzTransform {
                    let qpRaw: UInt16 = payload.withUnsafeBytes { $0.load(fromByteOffset: 2, as: UInt16.self) }
                    fp.qp = FilterParams.decodeQp(qpRaw)
                }
                if bands[band] != fp {
                    bands[band] = fp
                    self.channelData[ch] = bands
                    self.recomputeMagnitudes(for: ch)
                }
            }
            return
        }

        // Crossover band updates: 17 channels × 4 bands × 16 bytes at the V16
        // crossover offset (BULK_CROSSOVER_OFFSET = 4776).  Layout mirrors PEQ
        // but in its own WireCrossoverConfig section.  Input rows (ch < chOut1)
        // come through zeroed and we just store-but-don't-render them.
        let xoverBase = BULK_CROSSOVER_OFFSET
        let xoverChannelStride = WIRE_MAX_XOVER_BANDS * WIRE_BAND_PARAMS_SIZE
        if sz == WIRE_BAND_PARAMS_SIZE,
           off >= xoverBase,
           off < xoverBase + WIRE_MAX_CHANNELS * xoverChannelStride,
           (off - xoverBase) % WIRE_BAND_PARAMS_SIZE == 0,
           payload.count >= WIRE_BAND_PARAMS_SIZE {
            let flatIdx = (off - xoverBase) / WIRE_BAND_PARAMS_SIZE
            let ch = flatIdx / WIRE_MAX_XOVER_BANDS
            let localBand = flatIdx % WIRE_MAX_XOVER_BANDS
            if var bands = self.xoverData[ch], localBand < bands.count {
                let typeByte = payload[0]
                let bypassByte = payload[1]
                let freq: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 4, as: Float.self) }
                let q: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 8, as: Float.self) }
                let gain: Float = payload.withUnsafeBytes { $0.load(fromByteOffset: 12, as: Float.self) }
                var fp = bands[localBand]
                fp.type = FilterType(rawValue: Int(typeByte)) ?? .flat
                fp.freq = freq
                fp.q = q
                fp.gain = gain
                fp.bypass = bypassByte == 1
                if bands[localBand] != fp {
                    bands[localBand] = fp
                    self.xoverData[ch] = bands
                    self.recomputeMagnitudes(for: ch)
                }
                self.firmwareSupportsCrossover = true
            }
            return
        }

        // Channel names: 17 channels × 32 bytes at the V16 offset (4088).
        let channelNamesBase = BULK_CHANNEL_NAMES_OFFSET
        let channelNameSize = 32
        if sz == channelNameSize,
           off >= channelNamesBase,
           off < channelNamesBase + channelCount * channelNameSize,
           (off - channelNamesBase) % channelNameSize == 0,
           payload.count >= channelNameSize {
            let ch = (off - channelNamesBase) / channelNameSize
            let nulIdx = payload.firstIndex(of: 0) ?? payload.endIndex
            let slice = payload[payload.startIndex..<nulIdx]
            let name = String(data: slice, encoding: .utf8)
                ?? String(data: slice, encoding: .ascii)
                ?? ""
            self.channelNames[ch] = name
            return
        }

        // dac_hw_mute (V16 offset 4760, full 16-byte struct).  Emitted by the
        // firmware on a successful REQ_SET_DAC_HW_MUTE_CONFIG or after a
        // factory-reset rewrite of the directory.  Mirror back so the UI
        // stays in sync with non-HOST writes.
        if off == BULK_DAC_HW_MUTE_OFFSET && sz == 16 && payload.count >= 16,
           let cfg = DacHwMuteConfig.fromData(payload) {
            self.dacHwMuteConfig = cfg
            self.dacHwMuteSupported = true
            return
        }

        // user_volume.user_volume_db (BULK_USER_VOLUME_OFFSET = 4744, 4 bytes float dB).
        // Fired on REQ_SET_USER_VOLUME (PARAM_SRC_HOST_SET — ignored
        // upstream as our own echo), bulk apply, and — crucially — on
        // OS volume-slider moves via UAC1, which the firmware reports
        // with PARAM_SRC_UAC1.  Those UAC1 events are how the in-app
        // slider stays synced with the OS volume control during USB
        // playback (UAC1 can't push the value the other direction, so
        // this notification is the only signal we get).  GPIO/INTERNAL
        // (hardware-knob / firmware) writes flow through here too.
        if off == BULK_USER_VOLUME_OFFSET && sz == 4 && payload.count >= 4 {
            let db: Float = payload.withUnsafeBytes { $0.load(as: Float.self) }
            if abs(self.userVolumeDB - db) > 0.01 {
                self.userVolumeDB = db
            }
            return
        }

        // lg_sound_sync.enabled (BULK_LG_OFFSET = 4728, 1 byte).  Fired by
        // lg_sound_sync_set_enabled() on the firmware side, and on bulk
        // apply.  Mirrors the user-controlled gate so external hosts /
        // preset loads flow into our Settings toggle without a re-poll.
        if off == BULK_LG_OFFSET && sz == 1 && payload.count >= 1 {
            let en = payload[0] != 0
            if self.lgSoundSyncEnabled != en {
                self.lgSoundSyncEnabled = en
            }
            self.lgSoundSyncSupported = true
            return
        }

        // input_config.input_source (BULK_INPUT_CONFIG_OFFSET, 1 byte).  Fired at the
        // tail of the firmware's deferred input-source switch handler —
        // i.e. after all per-source thaw/init work has run (including
        // the SPDIF→USB audio_set_volume() thaw).  Use this as the
        // "switch complete" trigger to refresh user volume so the
        // slider lands on the firmware's authoritative value, regardless
        // of which direction we switched.
        if off == BULK_INPUT_CONFIG_OFFSET && sz == 1 && payload.count >= 1 {
            let src = Int(payload[0])
            if self.inputSource != src {
                self.inputSource = src
            }
            let inputChannelCount = self.numInputChannels
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.fetchUserVolume()
                // The firmware relabels default input-channel names to follow the
                // new source (e.g. "SPDIF L/R"); custom names are preserved.
                // Refetch the input rows so the sidebar/matrix reflect the change.
                for ch in 0..<inputChannelCount {
                    self?.fetchChannelName(channel: ch)
                }
            }
            return
        }

        // input_config.i2s_rx_pin (+2, 1 byte) — pair 0 data pin.
        // Fires when 0xF1 (pair 0) succeeds, bulk apply, or a preset load.
        if off == BULK_INPUT_CONFIG_OFFSET + 2 && sz == 1 && payload.count >= 1 {
            self.i2sInputSupported = true
            self.i2sRxPins[0] = payload[0]
            return
        }

        // input_config.i2s_input_rate (+3, 1 byte, enum 0/1/2) —
        // fires when 0xED accepts a rate.
        if off == BULK_INPUT_CONFIG_OFFSET + 3 && sz == 1 && payload.count >= 1 {
            self.i2sInputSupported = true
            self.i2sInputRateHz = i2sRateEnumToHz(payload[0])
            return
        }

        // input_config.i2s_input_channels (offset 4716, 1 byte: 2/4/6/8) —
        // fires when 0xF3 changes the active I2S channel count.
        if off == BULK_INPUT_CONFIG_OFFSET + 4 && sz == 1 && payload.count >= 1, payload[0] != 0 {
            self.i2sInputSupported = true
            self.i2sInputChannels = Int(payload[0])
            return
        }

        // input_config.i2s_rx_pin_ext[0..2] (offsets 4717..4719) — data pins for
        // stereo pairs 1..3.  Fires when 0xF1 (pair >= 1) succeeds, bulk, preset.
        if off >= BULK_INPUT_CONFIG_OFFSET + 5 && off <= BULK_INPUT_CONFIG_OFFSET + 7 && sz == 1 && payload.count >= 1 {
            let pair = off - (BULK_INPUT_CONFIG_OFFSET + 5) + 1   // 1..3
            self.i2sInputSupported = true
            if self.i2sRxPins.indices.contains(pair) { self.i2sRxPins[pair] = payload[0] }
            return
        }

        // input_config.i2s_clock_mode (offset 4727, 1 byte: 0=master/1=slave).
        // Fired at the tail of the firmware's deferred mode-change transition
        // (not at SET time) - i.e. the "mode is now live" confirmation.  A
        // NOTIFY 0x09 with the fresh lock state follows; refresh the full status.
        if off == BULK_INPUT_I2S_CLOCK_MODE_OFFSET && sz == 1 && payload.count >= 1 {
            let mode = payload[0]
            self.i2sClockModeSupported = true
            self.i2sInputSupported = true
            if self.i2sClockMode != mode { self.i2sClockMode = mode }
            self.i2sSlaveStatus.clockMode = mode
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.fetchI2SSlaveStatus()
            }
            return
        }

        // input_config.adat_input_pin (offset 4728, 1 byte: raw GPIO, 0 = unset).
        // Fires when 0x6A re-routes the RX pin, on bulk apply, and on preset load.
        if off == BULK_INPUT_ADAT_PIN_OFFSET && sz == 1 && payload.count >= 1 {
            let pin = payload[0] == 0 ? ADAT_INPUT_PIN_UNSET : payload[0]
            if self.adatInputPin != pin { self.adatInputPin = pin }
            self.adatInputStatus.pin = pin
            return
        }

        // input_config.adat_input_enabled_p1 (offset 4729, +1 encoded: 1=disabled,
        // 2=enabled).  Fires when 0x68 toggles the input, on bulk, and on preset.
        if off == BULK_INPUT_ADAT_ENABLED_P1_OFFSET && sz == 1 && payload.count >= 1, payload[0] != 0 {
            let en = payload[0] == 2
            if self.adatInputEnabled != en { self.adatInputEnabled = en }
            self.adatInputStatus.enabled = en
            return
        }

        // input_config.adat_clock_mode_p1 (offset 4730, +1 encoded: 1=master,
        // 2=slave).  Fired at the tail of the firmware's deferred clock-mode apply
        // (the "mode is now live" confirmation); a NOTIFY 0x0B follows, so refresh
        // the full status.
        if off == BULK_INPUT_ADAT_CLOCK_MODE_P1_OFFSET && sz == 1 && payload.count >= 1, payload[0] != 0 {
            let mode = payload[0] == 2 ? ADAT_INPUT_CLOCK_MODE_SLAVE : ADAT_INPUT_CLOCK_MODE_MASTER
            if self.adatInputClockMode != mode { self.adatInputClockMode = mode }
            self.adatInputStatus.clockMode = mode
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.fetchAdatInputStatus()
            }
            return
        }

        // i2s_config.bck_pin (BULK_I2S_OFFSET + 4, 1 byte).  Fired when 0xC2
        // role 0 re-routes the master/unified BCK pair, on bulk, and on preset.
        if off == BULK_I2S_OFFSET + 4 && sz == 1 && payload.count >= 1 {
            if self.i2sBckPin != payload[0] { self.i2sBckPin = payload[0] }
            return
        }

        // i2s_config.clock_pin_mode_p1 (BULK_I2S_CLOCK_PIN_MODE_OFFSET, 1 byte,
        // +1 encoded: 1=unified, 2=split).  Fired when 0xFE applies, on bulk, and
        // on preset load.  0xFE applies synchronously so this is a confirmation.
        if off == BULK_I2S_CLOCK_PIN_MODE_OFFSET && sz == 1 && payload.count >= 1, payload[0] != 0 {
            let mode = payload[0] - 1
            self.i2sClockPinModeSupported = true
            if self.i2sClockPinMode != mode { self.i2sClockPinMode = mode }
            return
        }

        // i2s_config.bck_pin_slave (BULK_I2S_BCK_PIN_SLAVE_OFFSET, 1 byte, plain
        // GPIO).  Fired when 0xC2 role 1 moves the slave pair, on bulk, preset.
        if off == BULK_I2S_BCK_PIN_SLAVE_OFFSET && sz == 1 && payload.count >= 1, payload[0] != 0 {
            if self.i2sBckPinSlave != payload[0] { self.i2sBckPinSlave = payload[0] }
            return
        }

        // adat_config.enabled (BULK_ADAT_OFFSET, 1 byte).  Fired when 0xCA
        // toggles the stream, on bulk apply, and on preset load.  The richer
        // live state (active / rate_ok) arrives via NOTIFY_EVT_ADAT_STATE and
        // fetchAdatStatus(); here we just mirror the persisted enable intent.
        if off == BULK_ADAT_OFFSET && sz == 1 && payload.count >= 1 {
            let en = payload[0] != 0
            if self.adatEnabled != en { self.adatEnabled = en }
            self.adatStatus.enabled = en
            return
        }

        // adat_config.pin (BULK_ADAT_OFFSET + 1, 1 byte).  Fired when 0xCC
        // re-routes the data pin, on bulk apply, and on preset load.  A stored
        // 0 means "unset — use the platform default".
        if off == BULK_ADAT_OFFSET + 1 && sz == 1 && payload.count >= 1 {
            let pin = payload[0] == 0 ? ADAT_PIN_DEFAULT : payload[0]
            if self.adatPin != pin { self.adatPin = pin }
            self.adatStatus.pin = pin
            return
        }

        // Not an offset we decode here.  Re-read the whole block instead, on a
        // longer delay than the preset path uses: a pot bound to one of these
        // parameters emits at the notification rate, and a quarter second of
        // coalescing turns a knob sweep into a handful of fetches.
        scheduleDeviceResync(delay: 0.25)
    }

    // MARK: - Flash Storage Commands

    func saveParams() -> UInt8 {
        guard isDeviceConnected else { return FLASH_ERR_WRITE }
        return transport.withDeviceLock(reason: "Saving to flash", timeout: 5) {
            transport.getControlRequest(request: REQ_SAVE_PARAMS, value: 0, index: 0, length: 1)?.first
        }.flatMap { $0 } ?? FLASH_ERR_WRITE
    }

    func loadParams() -> UInt8 {
        guard isDeviceConnected else { return FLASH_ERR_WRITE }
        // "Revert to Saved" reloads the active preset slot from flash,
        // discarding unsaved live edits.  Route it through the DEFERRED
        // REQ_PRESET_LOAD path (loadPreset) rather than a synchronous load: a
        // synchronous flash read + state apply inside the device's USB-IRQ
        // handler WITHOUT stopping the SPDIF receiver crashes the device when
        // SPDIF is the active input (its decode-timeout alarm tears down
        // DMA/PIO during the ~45 ms flash blackout).  loadPreset defers
        // on-device — it stops RX, fences Core 1, and resyncs the pipeline — so
        // it is safe on every input source, and it also refreshes the UI and
        // rebaselines the unsaved-changes snapshot.  (The legacy synchronous
        // load opcode 0x52 has since been repurposed as REQ_SAVE_OUTPUT_CONFIG.)
        let status = loadPreset(slot: activePresetSlot)
        // Map preset status codes onto the FLASH_* codes the caller expects.
        switch status {
        case PRESET_OK:      return FLASH_OK
        case PRESET_ERR_CRC: return FLASH_ERR_CRC
        default:             return FLASH_ERR_WRITE   // invalid slot / USB failure (0xFF)
        }
    }

    func factoryReset() -> UInt8 {
        guard isDeviceConnected else { return FLASH_ERR_WRITE }
        // fetchAll() below covers the rewrite; drop the device's own
        // BULK_INVALIDATED(source=FACTORY) so it doesn't duplicate that work.
        suppressDeviceResync()
        if let data = transport.getControlRequest(request: REQ_FACTORY_RESET, value: 0, index: 0, length: 1) {
            let result = data[0]
            if result == FLASH_OK {
                // Re-fetch all params to update UI
                fetchAll()
            }
            return result
        }
        return FLASH_ERR_WRITE
    }
}

// -- WP SW9 --
extension OutputLatencyOption {
    /// An option as the core decoded it, in this app's form.
    init(core o: dspi_core.OutputLatencyOption) {
        self.init(id: o.id,
                  flags: (o.streamOnly ? OUTPUT_LATENCY_FLAG_STREAM_ONLY : 0) | (o.available ? OUTPUT_LATENCY_WIRE_AVAILABLE : 0),
                  latencyUs: o.latencyUs, batchUs: o.batchUs, effectiveUs: o.effectiveUs)
    }
}
