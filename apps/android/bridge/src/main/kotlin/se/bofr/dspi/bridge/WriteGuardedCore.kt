// G1 (owner decision): the bridge's last line of defence for read-only mode.
package se.bofr.dspi.bridge

// -- WP P1AB, P2CC2, P2CC3, P3CC -- (one sorted block: ktlint import-ordering)
import se.bofr.dspi.core.AbCompare
import se.bofr.dspi.core.AbSide
import se.bofr.dspi.core.AlinkSceneRecall
import se.bofr.dspi.core.AlinkSystemAlign
import se.bofr.dspi.core.AlinkSystemEdits
import se.bofr.dspi.core.AlinkSystemSaved
import se.bofr.dspi.core.AutoEqApply
import se.bofr.dspi.core.AutoEqProfile
import se.bofr.dspi.core.ChannelClip
import se.bofr.dspi.core.ChannelPasteOptions
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.FilterParams
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.core.HistoryMove
import se.bofr.dspi.core.HubDevice
import se.bofr.dspi.core.HubFleetTarget
import se.bofr.dspi.core.HubGroupParams
import se.bofr.dspi.core.HubGroupRow
import se.bofr.dspi.core.HubGroupRun
import se.bofr.dspi.core.HubInventoryForgot
import se.bofr.dspi.core.HubLimitsSet
import se.bofr.dspi.core.HubOffset
import se.bofr.dspi.core.HubPresetPick
import se.bofr.dspi.core.HubRollout
import se.bofr.dspi.core.HubScheduleRun
import se.bofr.dspi.core.HubSnapshotRecall
import se.bofr.dspi.core.I2cConfig
import se.bofr.dspi.core.IdentifyInfo
import se.bofr.dspi.core.InstallationDraft
import se.bofr.dspi.core.LimitsApply
import se.bofr.dspi.core.LockState
import se.bofr.dspi.core.MeasureLiveSetup
import se.bofr.dspi.core.MeasureLiveState
import se.bofr.dspi.core.MeasureRolloffSetup
import se.bofr.dspi.core.MeasureRolloffState
import se.bofr.dspi.core.MeasureSetup
import se.bofr.dspi.core.MeasureState
import se.bofr.dspi.core.MeasureSystemApplied
import se.bofr.dspi.core.MeasureSystemSetup
import se.bofr.dspi.core.MeasureSystemState
import se.bofr.dspi.core.MeasureSystemUnitCheck
import se.bofr.dspi.core.MeasureTuneState
import se.bofr.dspi.core.ParamValue
import se.bofr.dspi.core.SiggenConfig
import se.bofr.dspi.core.SnapshotRecall
import se.bofr.dspi.core.SystemVolumePlan
import se.bofr.dspi.core.UartConfig
import se.bofr.dspi.core.VregDemoEvent

/**
 * G1: wraps the core so that no device WRITE / command can reach USB while
 * [allowed] is false, whatever path calls it. Every [DspiBridgeImpl] member
 * refuses up front (with its event and failure value); this layer only
 * catches a path that forgot to (then [trips] counts it and the call throws
 * [DspiException.Unsupported], which every caller already maps to its
 * failure value). Reads, local computations, listeners, RTA subscriptions,
 * control-surface `refresh`/`refreshStatus` and [enterBootloader] (the
 * firmware-update path) pass through.
 *
 * [WRITES] is the explicit classification of [DspiCoreApi]; a test fails
 * when a core method is in neither [WRITES] nor [PASS_THROUGH].
 */
internal class WriteGuardedCore(private val inner: DspiCoreApi, private val allowed: () -> Boolean) : DspiCoreApi by inner {
    /** Writes this layer had to stop (the bridge's own guard missed them). */
    @Volatile var trips = 0
        private set

    private fun guard(name: String) {
        if (!allowed()) {
            trips++
            throw DspiException.Unsupported()
        }
    }

    // -- WP AN1 --
    // Writes of the new core. It also refuses them by itself on a read-only
    // connection (DspiException.ReadOnly); this layer's rule stays the same.
    override fun setDeviceName(name: String) {
        guard("setDeviceName")
        return inner.setDeviceName(name)
    }

    override fun setInputSourceName(source: Int, name: String) {
        guard("setInputSourceName")
        return inner.setInputSourceName(source, name)
    }

    override fun setOutputLatency(id: Int) {
        guard("setOutputLatency")
        return inner.setOutputLatency(id)
    }

    // -- WP P1TZF -- the core-voltage operation's only form: a SET takes a token
    override fun coreVregOpConfirmed(op: Int, value: Int, word: String?, token: String?) {
        guard("coreVregOpConfirmed")
        return inner.coreVregOpConfirmed(op, value, word, token)
    }

    // -- WP FWX -- blinks the LED, and with a tone output plays a tone
    override fun identify(seconds: Int, toneOutput: Int?) {
        guard("identify")
        return inner.identify(seconds, toneOutput)
    }

    // -- WP SW1 -- the audio link's writes (slots, phase lock, apply at frame, counters)
    override fun setAlinkSlots(slots: List<Int>) {
        guard("setAlinkSlots")
        return inner.setAlinkSlots(slots)
    }

    override fun setAlinkPhase(mode: Int, dLocal: Int, hopsOffset: Int) {
        guard("setAlinkPhase")
        return inner.setAlinkPhase(mode, dLocal, hopsOffset)
    }

    override fun applyAtFrame(frame: Long, op: Int, arg: Int, value: Int) {
        guard("applyAtFrame")
        return inner.applyAtFrame(frame, op, arg, value)
    }

    override fun resetAlinkStats() {
        guard("resetAlinkStats")
        return inner.resetAlinkStats()
    }

    // -- WP AUSB1 -- the USB-direct block's writes (mode and D, DIAG clear, apply on a time base)
    override fun setAlinkUsb(mode: Int, autoReanchor: Boolean, d: Int) {
        guard("setAlinkUsb")
        return inner.setAlinkUsb(mode, autoReanchor, d)
    }

    override fun clearAlinkUsbDiag() {
        guard("clearAlinkUsbDiag")
        return inner.clearAlinkUsbDiag()
    }

    // -- WP AUSB2 -- the lock-error action (EXT_ALINK_USB byte 4)
    override fun setAlinkUsbOnLockError(playUnaligned: Boolean) {
        guard("setAlinkUsbOnLockError")
        return inner.setAlinkUsbOnLockError(playUnaligned)
    }

    override fun applyAtTimeBase(timeBase: Int, frame: Long, op: Int, arg: Int, value: Int) {
        guard("applyAtTimeBase")
        return inner.applyAtTimeBase(timeBase, frame, op, arg, value)
    }

    // -- WP LKUI1 -- the hub's audio link installation: saving the model, writing the
    // audio link delays and recalling a scene change devices (the hub judges the role too)
    override fun alinkSystemSave(revision: Long, edits: AlinkSystemEdits): Long {
        guard("alinkSystemSave")
        return inner.alinkSystemSave(revision, edits)
    }

    // -- WP P1DCFIX -- the save with the hub's warnings
    override fun alinkSystemSaveReport(revision: Long, edits: AlinkSystemEdits): AlinkSystemSaved {
        guard("alinkSystemSaveReport")
        return inner.alinkSystemSaveReport(revision, edits)
    }

    // -- WP INST0 -- the Installation setup's save: the hub's site model (system.set,
    // an Admin write), as alinkSystemSave
    override fun installationSave(draft: InstallationDraft): AlinkSystemSaved {
        guard("installationSave")
        return inner.installationSave(draft)
    }

    // -- WP FEAT2 -- a room's or zone's volume (group.param.set on its units)
    override fun systemVolumeSet(plan: SystemVolumePlan): SystemVolumePlan {
        guard("systemVolumeSet")
        return inner.systemVolumeSet(plan)
    }

    override fun alinkSystemAlign(): AlinkSystemAlign {
        guard("alinkSystemAlign")
        return inner.alinkSystemAlign()
    }

    override fun alinkSystemRecall(scene: String): AlinkSceneRecall {
        guard("alinkSystemRecall")
        return inner.alinkSystemRecall(scene)
    }

    // Review w1d (integration with CG11, CG14): the calibration flow's writes.
    // The core's own poll writes only the next step of a run that start began,
    // and start is guarded, so the poll passes through.
    override fun coreVregCalStart(target: Int, soakS: Long, typed: String?) {
        guard("coreVregCalStart")
        return inner.coreVregCalStart(target, soakS, typed)
    }

    override fun coreVregTrialEnd() {
        guard("coreVregTrialEnd")
        return inner.coreVregTrialEnd()
    }

    override fun coreVregManualTrial(value: Int, typed: String?) {
        guard("coreVregManualTrial")
        return inner.coreVregManualTrial(value, typed)
    }

    override fun coreVregManualAuto(on: Boolean) {
        guard("coreVregManualAuto")
        return inner.coreVregManualAuto(on)
    }

    override fun coreVregManualClear() {
        guard("coreVregManualClear")
        return inner.coreVregManualClear()
    }

    override fun coreVregDemoEvent(event: VregDemoEvent, seconds: Long) {
        guard("coreVregDemoEvent")
        return inner.coreVregDemoEvent(event, seconds)
    }

    override fun csDemoBusy(on: Boolean) {
        guard("csDemoBusy")
        return inner.csDemoBusy(on)
    }

    // -- WP CG20 --
    override fun csDemoLearn(protocol: Int, code: Long): Boolean {
        guard("csDemoLearn")
        return inner.csDemoLearn(protocol, code)
    }

    override fun demoRenameElsewhere(target: Int, name: String) {
        guard("demoRenameElsewhere")
        return inner.demoRenameElsewhere(target, name)
    }

    override fun demoLatencyTiming(mode: Int, reads: Int, blackout: Int) {
        guard("demoLatencyTiming")
        return inner.demoLatencyTiming(mode, reads, blackout)
    }

    // -- WP QD31 --
    override fun demoOutputSaveStatus(status: Int) {
        guard("demoOutputSaveStatus")
        return inner.demoOutputSaveStatus(status)
    }

    override fun rawSet(opcode: Int, value: Int, data: List<Int>) {
        guard("rawSet")
        return inner.rawSet(opcode, value, data)
    }

    // -- WP PGCORE -- a raw paged SET is a write, as rawSet
    override fun rawSetId(id: Int, value: Int, data: List<Int>) {
        guard("rawSetId")
        return inner.rawSetId(id, value, data)
    }

    override fun applyAllParams() {
        guard("applyAllParams")
        return inner.applyAllParams()
    }

    // -- WP P0W -- an undo or redo writes the device like the setter it replays
    override fun undo(): HistoryMove {
        guard("undo")
        return inner.undo()
    }

    override fun redo(): HistoryMove {
        guard("redo")
        return inner.redo()
    }

    // -- WP MCP0 -- reverting one change by id replays it to the device, as undo does
    override fun revertEntry(id: Long): HistoryMove {
        guard("revertEntry")
        return inner.revertEntry(id)
    }

    override fun setFilter(ch: Int, band: Int, params: FilterParams) {
        guard("setFilter")
        return inner.setFilter(ch, band, params)
    }

    override fun setCrossover(ch: Int, xband: Int, params: FilterParams) {
        guard("setCrossover")
        return inner.setCrossover(ch, xband, params)
    }

    override fun setBandBypass(ch: Int, band: Int, bypass: Boolean) {
        guard("setBandBypass")
        return inner.setBandBypass(ch, band, bypass)
    }

    override fun setPreamp(db: Float) {
        guard("setPreamp")
        return inner.setPreamp(db)
    }

    override fun setInputPreamp(input: Int, db: Float) {
        guard("setInputPreamp")
        return inner.setInputPreamp(input, db)
    }

    override fun setBypass(enabled: Boolean) {
        guard("setBypass")
        return inner.setBypass(enabled)
    }

    override fun setMasterVolume(db: Float) {
        guard("setMasterVolume")
        return inner.setMasterVolume(db)
    }

    override fun setMasterVolumeMode(mode: Int) {
        guard("setMasterVolumeMode")
        return inner.setMasterVolumeMode(mode)
    }

    override fun saveMasterVolume() {
        guard("saveMasterVolume")
        return inner.saveMasterVolume()
    }

    override fun setUserVolume(db: Float) {
        guard("setUserVolume")
        return inner.setUserVolume(db)
    }

    override fun setUserMute(muted: Boolean) {
        guard("setUserMute")
        return inner.setUserMute(muted)
    }

    override fun setDelay(ch: Int, ms: Float) {
        guard("setDelay")
        return inner.setDelay(ch, ms)
    }

    override fun setLoudness(enabled: Boolean) {
        guard("setLoudness")
        return inner.setLoudness(enabled)
    }

    override fun setLoudnessRef(spl: Float) {
        guard("setLoudnessRef")
        return inner.setLoudnessRef(spl)
    }

    override fun setLoudnessIntensity(pct: Float) {
        guard("setLoudnessIntensity")
        return inner.setLoudnessIntensity(pct)
    }

    override fun setLoudnessMask(mask: Int) {
        guard("setLoudnessMask")
        return inner.setLoudnessMask(mask)
    }

    override fun setCrossfeed(enabled: Boolean) {
        guard("setCrossfeed")
        return inner.setCrossfeed(enabled)
    }

    override fun setCrossfeedPreset(preset: Int) {
        guard("setCrossfeedPreset")
        return inner.setCrossfeedPreset(preset)
    }

    override fun setCrossfeedFreq(freq: Float) {
        guard("setCrossfeedFreq")
        return inner.setCrossfeedFreq(freq)
    }

    override fun setCrossfeedFeed(feed: Float) {
        guard("setCrossfeedFeed")
        return inner.setCrossfeedFeed(feed)
    }

    override fun setCrossfeedItd(enabled: Boolean) {
        guard("setCrossfeedItd")
        return inner.setCrossfeedItd(enabled)
    }

    override fun setCrossfeedOutputs(pairMask: Int) {
        guard("setCrossfeedOutputs")
        return inner.setCrossfeedOutputs(pairMask)
    }

    override fun setCrossfeedPairMap(map: List<Int>) {
        guard("setCrossfeedPairMap")
        return inner.setCrossfeedPairMap(map)
    }

    override fun setLevellerEnabled(enabled: Boolean) {
        guard("setLevellerEnabled")
        return inner.setLevellerEnabled(enabled)
    }

    override fun setLevellerAmount(pct: Float) {
        guard("setLevellerAmount")
        return inner.setLevellerAmount(pct)
    }

    override fun setLevellerSpeed(speed: Int) {
        guard("setLevellerSpeed")
        return inner.setLevellerSpeed(speed)
    }

    override fun setLevellerMaxGain(db: Float) {
        guard("setLevellerMaxGain")
        return inner.setLevellerMaxGain(db)
    }

    override fun setLevellerLookahead(enabled: Boolean) {
        guard("setLevellerLookahead")
        return inner.setLevellerLookahead(enabled)
    }

    override fun setLevellerGate(db: Float) {
        guard("setLevellerGate")
        return inner.setLevellerGate(db)
    }

    override fun setLevellerMasks(detector: Int, apply: Int) {
        guard("setLevellerMasks")
        return inner.setLevellerMasks(detector, apply)
    }

    override fun setInputSource(source: Int) {
        guard("setInputSource")
        return inner.setInputSource(source)
    }

    override fun setPsybassParam(param: Int, value: Float) {
        guard("setPsybassParam")
        return inner.setPsybassParam(param, value)
    }

    override fun setPsybassMask(mask: Int) {
        guard("setPsybassMask")
        return inner.setPsybassMask(mask)
    }

    override fun setPsybassEnabled(enabled: Boolean) {
        guard("setPsybassEnabled")
        return inner.setPsybassEnabled(enabled)
    }

    override fun setMatrixRoute(input: Int, output: Int, enabled: Boolean, gain: Float, invert: Boolean) {
        guard("setMatrixRoute")
        return inner.setMatrixRoute(input, output, enabled, gain, invert)
    }

    override fun setOutputEnable(output: Int, enabled: Boolean): Boolean {
        guard("setOutputEnable")
        return inner.setOutputEnable(output, enabled)
    }

    override fun setOutputGain(output: Int, db: Float) {
        guard("setOutputGain")
        return inner.setOutputGain(output, db)
    }

    override fun setOutputMute(output: Int, muted: Boolean) {
        guard("setOutputMute")
        return inner.setOutputMute(output, muted)
    }

    override fun setOutputDelay(output: Int, ms: Float) {
        guard("setOutputDelay")
        return inner.setOutputDelay(output, ms)
    }

    override fun setLimiterParam(output: Int, param: Int, value: Float) {
        guard("setLimiterParam")
        return inner.setLimiterParam(output, param, value)
    }

    override fun setOutputPin(slot: Int, pin: Int) {
        guard("setOutputPin")
        return inner.setOutputPin(slot, pin)
    }

    override fun setOutputConfigMode(mode: Int) {
        guard("setOutputConfigMode")
        return inner.setOutputConfigMode(mode)
    }

    override fun setChannelName(ch: Int, name: String) {
        guard("setChannelName")
        return inner.setChannelName(ch, name)
    }

    override fun loadPreset(slot: Int) {
        guard("loadPreset")
        return inner.loadPreset(slot)
    }

    override fun setPresetName(slot: Int, name: String) {
        guard("setPresetName")
        return inner.setPresetName(slot, name)
    }

    override fun setPresetStartup(mode: Int, defaultSlot: Int) {
        guard("setPresetStartup")
        return inner.setPresetStartup(mode, defaultSlot)
    }

    override fun loadParams() {
        guard("loadParams")
        return inner.loadParams()
    }

    // -- WP AN6 -- Meter housekeeping, which the core allows on a read-only connection: the status
    // poll needs it, or the sticky flags would hold a clip marker on forever.
    override fun clearClips() = inner.clearClips()

    override fun setUpmixParam(id: Int, value: Float) {
        guard("setUpmixParam")
        return inner.setUpmixParam(id, value)
    }

    override fun setSubharmParam(id: Int, value: Float) {
        guard("setSubharmParam")
        return inner.setSubharmParam(id, value)
    }

    override fun setSubharmSolo(solo: Boolean) {
        guard("setSubharmSolo")
        return inner.setSubharmSolo(solo)
    }

    override fun setTubeParam(idx: Int, value: Float) {
        guard("setTubeParam")
        return inner.setTubeParam(idx, value)
    }

    override fun setOutputType(slot: Int, kind: Int) {
        guard("setOutputType")
        return inner.setOutputType(slot, kind)
    }

    override fun setI2sBckPin(role: Int, pin: Int) {
        guard("setI2sBckPin")
        return inner.setI2sBckPin(role, pin)
    }

    override fun setI2sClockPinMode(mode: Int) {
        guard("setI2sClockPinMode")
        return inner.setI2sClockPinMode(mode)
    }

    override fun setMckEnabled(enabled: Boolean) {
        guard("setMckEnabled")
        return inner.setMckEnabled(enabled)
    }

    override fun setMckPin(pin: Int) {
        guard("setMckPin")
        return inner.setMckPin(pin)
    }

    override fun setMckMultiplier(mult: Int) {
        guard("setMckMultiplier")
        return inner.setMckMultiplier(mult)
    }

    override fun setAdatOutEnabled(enabled: Boolean) {
        guard("setAdatOutEnabled")
        return inner.setAdatOutEnabled(enabled)
    }

    override fun setAdatOutPin(pin: Int) {
        guard("setAdatOutPin")
        return inner.setAdatOutPin(pin)
    }

    override fun setSpdifRxPin(index: Int, pin: Int) {
        guard("setSpdifRxPin")
        return inner.setSpdifRxPin(index, pin)
    }

    override fun setSpdifInputEnabled(index: Int, enabled: Boolean) {
        guard("setSpdifInputEnabled")
        return inner.setSpdifInputEnabled(index, enabled)
    }

    override fun setI2sRxPin(pair: Int, pin: Int) {
        guard("setI2sRxPin")
        return inner.setI2sRxPin(pair, pin)
    }

    override fun setI2sInputChannels(channels: Int) {
        guard("setI2sInputChannels")
        return inner.setI2sInputChannels(channels)
    }

    override fun setAdatInputEnabled(enabled: Boolean) {
        guard("setAdatInputEnabled")
        return inner.setAdatInputEnabled(enabled)
    }

    override fun setAdatInputPin(pin: Int) {
        guard("setAdatInputPin")
        return inner.setAdatInputPin(pin)
    }

    override fun setAdatInputClockMode(mode: Int) {
        guard("setAdatInputClockMode")
        return inner.setAdatInputClockMode(mode)
    }

    override fun setInputRate(index: Int) {
        guard("setInputRate")
        return inner.setInputRate(index)
    }

    override fun setI2sClockMode(mode: Int) {
        guard("setI2sClockMode")
        return inner.setI2sClockMode(mode)
    }

    override fun setDacMute(enabled: Boolean, activeLow: Boolean, pin: Int, holdMs: Int, releaseMs: Int) {
        guard("setDacMute")
        return inner.setDacMute(enabled, activeLow, pin, holdMs, releaseMs)
    }

    override fun testDacMute() {
        guard("testDacMute")
        return inner.testDacMute()
    }

    override fun setLgSoundSync(enabled: Boolean) {
        guard("setLgSoundSync")
        return inner.setLgSoundSync(enabled)
    }

    override fun setUart(config: UartConfig) {
        guard("setUart")
        return inner.setUart(config)
    }

    override fun setI2c(config: I2cConfig) {
        guard("setI2c")
        return inner.setI2c(config)
    }

    override fun siggenSetConfig(config: SiggenConfig) {
        guard("siggenSetConfig")
        return inner.siggenSetConfig(config)
    }

    override fun siggenControl(action: Int) {
        guard("siggenControl")
        return inner.siggenControl(action)
    }

    override fun resetBufferStats() {
        guard("resetBufferStats")
        return inner.resetBufferStats()
    }

    // -- WP P1TK -- the confirmed forms of the gated writes (the token is the
    // core's; this layer's read-only rule is the same as for the other writes; WP P1TZ retired the
    // forms without a token)
    override fun savePresetConfirmed(slot: Int, token: String?) {
        guard("savePresetConfirmed")
        return inner.savePresetConfirmed(slot, token)
    }

    override fun copyPresetConfirmed(source: Int, dest: Int, token: String?) {
        guard("copyPresetConfirmed")
        return inner.copyPresetConfirmed(source, dest, token)
    }

    override fun deletePresetConfirmed(slot: Int, token: String?) {
        guard("deletePresetConfirmed")
        return inner.deletePresetConfirmed(slot, token)
    }

    override fun saveParamsConfirmed(token: String?) {
        guard("saveParamsConfirmed")
        return inner.saveParamsConfirmed(token)
    }

    override fun factoryResetConfirmed(token: String?) {
        guard("factoryResetConfirmed")
        return inner.factoryResetConfirmed(token)
    }

    override fun saveOutputConfigConfirmed(token: String?) {
        guard("saveOutputConfigConfirmed")
        return inner.saveOutputConfigConfirmed(token)
    }

    override fun configImportConfirmed(text: String, options: Long, eqChannels: Long, token: String?): String {
        guard("configImportConfirmed")
        return inner.configImportConfirmed(text, options, eqChannels, token)
    }

    override fun coreVregCalCommitConfirmed(typed: String?, token: String?) {
        guard("coreVregCalCommitConfirmed")
        return inner.coreVregCalCommitConfirmed(typed, token)
    }

    override fun coreVregManualSetConfirmed(value: Int, typed: String?, token: String?) {
        guard("coreVregManualSetConfirmed")
        return inner.coreVregManualSetConfirmed(value, typed, token)
    }

    // -- WP P1SN -- a recall writes the device (names, latency, surfaces, the image)
    override fun snapshotRecall(id: String): SnapshotRecall {
        guard("snapshotRecall")
        return inner.snapshotRecall(id)
    }

    override fun snapshotRecallDocument(text: String): SnapshotRecall {
        guard("snapshotRecallDocument")
        return inner.snapshotRecallDocument(text)
    }

    // -- WP P5SES -- a measurement plays the generator: its start is a write
    // (the session's own steps, its stop and its aborts follow from it)
    override fun measureStart(setup: MeasureSetup): MeasureState {
        guard("measureStart")
        return inner.measureStart(setup)
    }

    // -- WP P5LIVE -- the live view plays the generator (its start) and
    // writes the previewed change (its apply)
    override fun measureLiveStart(setup: MeasureLiveSetup): MeasureLiveState {
        guard("measureLiveStart")
        return inner.measureLiveStart(setup)
    }

    override fun measureLiveApply(proposalId: Long): MeasureLiveState {
        guard("measureLiveApply")
        return inner.measureLiveApply(proposalId)
    }

    // -- WP P5TUNE -- the summed check and the verify play the generator, the
    // apply writes the proposal, the A/B start recalls a snapshot
    override fun measureTuneSumCheck(proposalId: Long): MeasureTuneState {
        guard("measureTuneSumCheck")
        return inner.measureTuneSumCheck(proposalId)
    }

    override fun measureTuneApply(proposalId: Long, guardS: Long): GuardedApply {
        guard("measureTuneApply")
        return inner.measureTuneApply(proposalId, guardS)
    }

    override fun measureTuneVerify(): MeasureTuneState {
        guard("measureTuneVerify")
        return inner.measureTuneVerify()
    }

    override fun measureTuneAb(): AbCompare {
        guard("measureTuneAb")
        return inner.measureTuneAb()
    }

    // -- WP ROLL1 -- the roll-off search plays the generator: its start is a
    // write (its segments, its stop and its aborts follow from it)
    override fun measureRolloffStart(setup: MeasureRolloffSetup): MeasureRolloffState {
        guard("measureRolloffStart")
        return inner.measureRolloffStart(setup)
    }

    // -- WP ATCORE -- the system measurement's start, summed check and verify
    // play the units' generators through the hub; its apply writes the hub's
    // model and every unit's output delays
    override fun measureSystemStart(setup: MeasureSystemSetup, checks: List<MeasureSystemUnitCheck>): MeasureSystemState {
        guard("measureSystemStart")
        return inner.measureSystemStart(setup, checks)
    }

    override fun measureSystemSumCheck(checks: List<MeasureSystemUnitCheck>): MeasureSystemState {
        guard("measureSystemSumCheck")
        return inner.measureSystemSumCheck(checks)
    }

    override fun measureSystemVerify(checks: List<MeasureSystemUnitCheck>): MeasureSystemState {
        guard("measureSystemVerify")
        return inner.measureSystemVerify(checks)
    }

    override fun measureSystemApply(proposalId: Long): MeasureSystemApplied {
        guard("measureSystemApply")
        return inner.measureSystemApply(proposalId)
    }

    // -- WP P1AB -- a start with A given, a toggle and a keep recall a snapshot
    override fun abStart(aId: String, bId: String): AbCompare {
        guard("abStart")
        return inner.abStart(aId, bId)
    }

    override fun abStartDocuments(aText: String, bText: String): AbCompare {
        guard("abStartDocuments")
        return inner.abStartDocuments(aText, bText)
    }

    override fun abToggle(): AbCompare {
        guard("abToggle")
        return inner.abToggle()
    }

    override fun abKeep(side: AbSide): AbCompare {
        guard("abKeep")
        return inner.abKeep(side)
    }

    // -- WP P1AQ -- an AutoEQ apply writes both channels of the pair
    override fun autoeqApply(profile: AutoEqProfile, left: Int, right: Int, applyLevel: Boolean?): AutoEqApply {
        guard("autoeqApply")
        return inner.autoeqApply(profile, left, right, applyLevel)
    }

    // -- WP P1LM -- applying a profile holds the device to it (writes)
    override fun limitsApplyProfile(document: String): LimitsApply {
        guard("limitsApplyProfile")
        return inner.limitsApplyProfile(document)
    }

    // -- WP P1LK -- what the lock covers is kept in the profile, which is then
    // applied (writes, as limitsApplyProfile)
    override fun installerLockCover(pages: List<String>): LockState {
        guard("installerLockCover")
        return inner.installerLockCover(pages)
    }

    // -- WP HUBADMIN -- the hub holds the device to a new profile at once (writes)
    override fun hubLimitsSet(document: String): HubLimitsSet {
        guard("hubLimitsSet")
        return inner.hubLimitsSet(document)
    }

    // -- WP P1SA -- a guarded import or recall writes the device
    override fun applyGuardedImport(text: String, options: Long, eqChannels: Long, timeoutS: Long): GuardedApply {
        guard("applyGuardedImport")
        return inner.applyGuardedImport(text, options, eqChannels, timeoutS)
    }

    override fun applyGuardedSnapshot(id: String, timeoutS: Long): GuardedApply {
        guard("applyGuardedSnapshot")
        return inner.applyGuardedSnapshot(id, timeoutS)
    }

    override fun applyGuardedSnapshotDocument(text: String, timeoutS: Long): GuardedApply {
        guard("applyGuardedSnapshotDocument")
        return inner.applyGuardedSnapshotDocument(text, timeoutS)
    }

    // -- WP P2CC1 -- Identify through the hub (or the device's own EXT_IDENTIFY
    // directly) blinks the LED and may play a tone; a recall on the hub writes
    // the device as this session
    override fun hubIdentify(seconds: Int, toneOutput: Int?): IdentifyInfo? {
        guard("hubIdentify")
        return inner.hubIdentify(seconds, toneOutput)
    }

    override fun hubSnapshotsRecall(serial: String, id: String, guard: Boolean, timeoutMs: Long): HubSnapshotRecall {
        this.guard("hubSnapshotsRecall") // `guard` here is the parameter
        return inner.hubSnapshotsRecall(serial, id, guard, timeoutMs)
    }

    // -- WP P2CC2 -- a schedule run now writes its target devices (as the
    // schedule's principal, on the hub)
    override fun hubScheduleRunNow(id: String): HubScheduleRun {
        guard("hubScheduleRunNow")
        return inner.hubScheduleRunNow(id)
    }

    // -- WP P2CC3 -- the hub's device and name writes: Identify of any device
    // on the hub blinks it (and may play a tone); the inventory's edits, a
    // forget and a rename change what the hub keeps
    override fun hubInventorySet(serial: String, location: String?, note: String?, tags: List<String>?): HubDevice {
        guard("hubInventorySet")
        return inner.hubInventorySet(serial, location, note, tags)
    }

    override fun hubInventoryForget(serial: String, withData: Boolean): HubInventoryForgot {
        guard("hubInventoryForget")
        return inner.hubInventoryForget(serial, withData)
    }

    override fun hubRename(name: String): String {
        guard("hubRename")
        return inner.hubRename(name)
    }

    override fun hubIdentifySerial(serial: String, seconds: Int, toneOutput: Int?): IdentifyInfo? {
        guard("hubIdentifySerial")
        return inner.hubIdentifySerial(serial, seconds, toneOutput)
    }

    // -- WP P3CC -- the hub's fleet writes: a group write, a group's preset
    // load and a bulk apply write every member (as this console's child
    // session); a rollout flashes them. Confirm and cancel of a run, a
    // rollout's cancel, groups and the image store pass (as applyConfirm,
    // applyRevert and the hub's snapshot store; the core's read-only mode
    // and the hub's roles gate them)
    override fun hubGroupParamSet(
        target: HubFleetTarget,
        path: String,
        idx: List<Long>,
        value: ParamValue,
        offsets: List<HubOffset>
    ): HubGroupParams {
        guard("hubGroupParamSet")
        return inner.hubGroupParamSet(target, path, idx, value, offsets)
    }

    override fun hubGroupPresetLoad(target: HubFleetTarget, slot: Int?, name: String, map: List<HubPresetPick>): List<HubGroupRow> {
        guard("hubGroupPresetLoad")
        return inner.hubGroupPresetLoad(target, slot, name, map)
    }

    override fun hubGroupApply(
        target: HubFleetTarget,
        snapshot: String,
        name: String,
        document: String,
        guard: Boolean,
        timeoutMs: Long
    ): HubGroupRun {
        this.guard("hubGroupApply") // `guard` here is the parameter
        return inner.hubGroupApply(target, snapshot, name, document, guard, timeoutMs)
    }

    override fun hubRolloutStart(target: HubFleetTarget, image: String, whileLive: Boolean): HubRollout {
        guard("hubRolloutStart")
        return inner.hubRolloutStart(target, image, whileLive)
    }

    override fun filtersImport(text: String, wires: List<Int>): String {
        guard("filtersImport")
        return inner.filtersImport(text, wires)
    }

    // -- WP P1PR --
    override fun setInputPairPreamp(input: Int, db: Float) {
        guard("setInputPairPreamp")
        return inner.setInputPairPreamp(input, db)
    }

    // -- WP PAIRBAND --
    override fun setInputPairFilter(input: Int, band: Int, params: FilterParams) {
        guard("setInputPairFilter")
        return inner.setInputPairFilter(input, band, params)
    }

    override fun setInputPairBandBypass(input: Int, band: Int, bypass: Boolean) {
        guard("setInputPairBandBypass")
        return inner.setInputPairBandBypass(input, band, bypass)
    }

    override fun protectAllOutputs(ceilingDbfs: Float) {
        guard("protectAllOutputs")
        return inner.protectAllOutputs(ceilingDbfs)
    }

    // -- WP CLIP1 -- a paste writes the destination (and its partner)
    override fun pasteChannel(clip: ChannelClip, ch: Int, options: ChannelPasteOptions) {
        guard("pasteChannel")
        return inner.pasteChannel(clip, ch, options)
    }

    override fun csApply(op: String): String {
        if (!isCsRead(op)) guard("csApply")
        return inner.csApply(op)
    }

    companion object {
        /** Core methods that write to the device or command it (refused while read-only). */
        val WRITES: Set<String> = setOf(
            "applyAllParams",
            "setFilter",
            "setCrossover",
            "setBandBypass",
            "setPreamp",
            "setInputPreamp",
            "setBypass",
            "setMasterVolume",
            "setMasterVolumeMode",
            "saveMasterVolume",
            "setUserVolume",
            "setUserMute",
            "setDelay",
            "setLoudness",
            "setLoudnessRef",
            "setLoudnessIntensity",
            "setLoudnessMask",
            "setCrossfeed",
            "setCrossfeedPreset",
            "setCrossfeedFreq",
            "setCrossfeedFeed",
            "setCrossfeedItd",
            "setCrossfeedOutputs",
            "setCrossfeedPairMap",
            "setLevellerEnabled",
            "setLevellerAmount",
            "setLevellerSpeed",
            "setLevellerMaxGain",
            "setLevellerLookahead",
            "setLevellerGate",
            "setLevellerMasks",
            "setInputSource",
            "setPsybassParam",
            "setPsybassMask",
            "setPsybassEnabled",
            "setMatrixRoute",
            "setOutputEnable",
            "setOutputGain",
            "setOutputMute",
            "setOutputDelay",
            "setLimiterParam",
            "setOutputPin",
            "setOutputConfigMode",
            "setChannelName",
            "loadPreset",
            "setPresetName",
            "setPresetStartup",
            "loadParams",
            "setUpmixParam",
            "setSubharmParam",
            "setSubharmSolo",
            "setTubeParam",
            "setOutputType",
            "setI2sBckPin",
            "setI2sClockPinMode",
            "setMckEnabled",
            "setMckPin",
            "setMckMultiplier",
            "setAdatOutEnabled",
            "setAdatOutPin",
            "setSpdifRxPin",
            "setSpdifInputEnabled",
            "setI2sRxPin",
            "setI2sInputChannels",
            "setAdatInputEnabled",
            "setAdatInputPin",
            "setAdatInputClockMode",
            "setInputRate",
            "setI2sClockMode",
            "setDacMute",
            "testDacMute",
            "setLgSoundSync",
            "setUart",
            "setI2c",
            "siggenSetConfig",
            "siggenControl",
            "resetBufferStats",
            "filtersImport",
            "csApply",
            // -- WP AN1 --
            "setDeviceName",
            "setInputSourceName",
            "setOutputLatency",
            "coreVregOpConfirmed",
            "rawSet",
            // -- WP P0W --
            "undo",
            "redo",
            // Review w1d (CG11, CG14): the calibration flow's device writes
            "coreVregCalStart",
            "coreVregTrialEnd",
            "coreVregManualTrial",
            "coreVregManualAuto",
            "coreVregManualClear",
            "coreVregDemoEvent",
            "csDemoBusy",
            // -- WP CG20 --
            "csDemoLearn",
            "demoRenameElsewhere",
            "demoLatencyTiming",
            // -- WP QD31 --
            "demoOutputSaveStatus",
            // -- WP FWX --
            "identify",
            // -- WP SW1 --
            "setAlinkSlots",
            "setAlinkPhase",
            "applyAtFrame",
            "resetAlinkStats",
            // -- WP AUSB1 --
            "setAlinkUsb",
            "clearAlinkUsbDiag",
            "applyAtTimeBase",
            // -- WP AUSB2 --
            "setAlinkUsbOnLockError",
            // -- WP LKUI1 --
            "alinkSystemSave",
            "alinkSystemAlign",
            "alinkSystemRecall",
            // -- WP P1DCFIX --
            "alinkSystemSaveReport",
            // -- WP INST0 -- the Installation setup's save (system.set, Admin)
            "installationSave",
            // -- WP FEAT2 -- a room's or zone's volume (group.param.set on its units)
            "systemVolumeSet",
            // -- WP MCP0 -- one change reverted by id: replays to the device as undo
            "revertEntry",
            // -- WP P1DF -- (configImportSelected retired by WP P1TZ)
            // -- WP P1PR --
            "setInputPairPreamp",
            "protectAllOutputs",
            // -- WP PAIRBAND --
            "setInputPairFilter",
            "setInputPairBandBypass",
            // -- WP P1TK --
            "savePresetConfirmed",
            "copyPresetConfirmed",
            "deletePresetConfirmed",
            "saveParamsConfirmed",
            "factoryResetConfirmed",
            "saveOutputConfigConfirmed",
            "configImportConfirmed",
            "coreVregCalCommitConfirmed",
            "coreVregManualSetConfirmed",
            // -- WP P1SN --
            "snapshotRecall",
            "snapshotRecallDocument",
            // -- WP P1AB --
            "abStart",
            "abStartDocuments",
            "abToggle",
            "abKeep",
            // -- WP P1LM --
            "limitsApplyProfile",
            // -- WP P1LK --
            "installerLockCover",
            // -- WP HUBADMIN --
            "hubLimitsSet",
            // -- WP P1SA --
            "applyGuardedImport",
            "applyGuardedSnapshot",
            "applyGuardedSnapshotDocument",
            // -- WP P1AQ --
            "autoeqApply",
            // -- WP CLIP1 --
            "pasteChannel",
            // -- WP P2CC1 --
            "hubIdentify",
            "hubSnapshotsRecall",
            // -- WP P2CC2 --
            "hubScheduleRunNow",
            // -- WP P2CC3 --
            "hubInventorySet",
            "hubInventoryForget",
            "hubRename",
            "hubIdentifySerial",
            // -- WP P3CC --
            "hubGroupParamSet",
            "hubGroupPresetLoad",
            "hubGroupApply",
            "hubRolloutStart",
            // -- WP P5SES --
            "measureStart",
            // -- WP P5LIVE --
            "measureLiveStart",
            "measureLiveApply",
            // -- WP P5TUNE --
            "measureTuneSumCheck",
            "measureTuneApply",
            "measureTuneVerify",
            "measureTuneAb",
            // -- WP ATCORE --
            "measureSystemStart",
            "measureSystemSumCheck",
            "measureSystemVerify",
            "measureSystemApply",
            // -- WP PGCORE -- a raw paged SET
            "rawSetId",
            // -- WP ROLL1 --
            "measureRolloffStart"
        )

        /** Core methods that only read, compute locally, manage listeners or the connection, or reboot into the bootloader. */
        val PASS_THROUGH: Set<String> = setOf(
            "cachedInputSourceName",
            "cachedDeviceName",
            "demoLatencySets",
            "wireFeatures",
            // -- WP QPI2 -- the connect probe's answer, a read
            "featurePresent",
            "clearClips",
            // DSPi Link session management (no device writes; the hub's
            // role check and the bridge's remote read-only flag gate writes)
            "linkConnect",
            "linkPair",
            "linkDevices",
            "linkStatus",
            "linkSelectDevice",
            "linkClose",
            "setLinkListener",
            "linkParseAddress",
            "linkWsUrl",
            "linkFetchInfo",
            // -- WP SECCORE -- TLS-first connect, its state, and two parsers
            "linkConnectAdvertised",
            "linkSecurity",
            "linkParsePairLink",
            "linkCredentialHubKey",
            // -- WP PINTLS -- two pure functions over a stored credential
            "linkCredentialUnpaired",
            "linkCredentialTlsOnly",
            // -- WP PINX -- a pure function over a stored credential
            "linkCredentialHasToken",
            // -- WP LKFAST -- the core's DSPi Link hub memory: a local file, no device traffic
            "linkSetDir",
            "linkStartHub",
            "linkForgetLastHub",
            "linkRecheckTls",
            // -- WP LKFAST -- a hub's / the session's encryption state (words: the notice table)
            "linkHubEncryption",
            "linkSessionEncryption",
            "linkEncryptionNoticeKey",
            // -- WP LKFAST3 -- the hub memory's Forget (a local file); the start's dial is a dial, as linkConnect
            "linkForgetHub",
            "linkConnectAtStart",
            "bootloaderCounts",
            "captureBaseline",
            "checkCore1Conflict",
            // -- WP CG36 -- the same question, with the core's words: a read.
            "core1ConflictReason",
            "closeFd",
            "computeMagnitudeCurve",
            "computePhaseCurve",
            "computeResponse",
            "configExport",
            "configInspect",
            // -- WP P1DF -- the diff and the preview write nothing
            "configDiff",
            "configPreview",
            // -- WP P1AQ -- the AutoEQ preview reads the mirror only
            "autoeqPreview",
            // -- WP P1SN -- the store is the app's files; a capture only reads the device
            "snapshotSetDir",
            "snapshotCapture",
            "snapshotCaptureDocument",
            "snapshotList",
            "snapshotRename",
            "snapshotSetNote",
            "snapshotDelete",
            "snapshotExport",
            "snapshotImport",
            // -- WP P1AB -- the diff of two documents and the compare's state: no I/O
            "abDiff",
            "abState",
            // -- WP P1SA -- a begin only captures (the flow's writes are guarded one
            // by one); Keep writes no device; a revert (Revert, or the timeout on the
            // poll) puts back a state captured before and is never refused
            // (PLAN-CLIENTS 5.9); the status and the default are local
            "applyGuardBegin",
            "applyConfirm",
            "applyRevert",
            "applyPoll",
            "applyStatus",
            "applyGuardDefault",
            // -- WP P1OF -- a document opened offline is the demo device seeded
            // from it (as openDemo); saving and the templates write no device
            "openDocument",
            "offlineDocument",
            "saveDocument",
            "saveAsTemplate",
            "templateList",
            "templateText",
            // -- WP P1AFIX -- the stored custom pair, from the mirror: a read
            "crossfeedCustomPair",
            "crossfeedPairMapValid",
            "coreInfo",
            // -- WP CG20 -- The editing rules: from the last read, no device I/O.
            "csDescribe",
            "csEdit",
            "csNewBinding",
            "csNewRecord",
            "csStatusText",
            "csFetch",
            "csPinUses",
            // -- WP CG-SWS-b -- the pin map: from the mirror, no device I/O
            "pinMap",
            "pinCheck",
            "pinHolder",
            "csSnapshot",
            "curveFreq",
            "disconnect",
            "fetchAdatOutStatus",
            "fetchAll",
            "fetchBufferStats",
            "fetchCore1Mode",
            "fetchCrossfeedPairMap",
            "fetchCtrlIfaces",
            "fetchDacMute",
            "fetchInputLock",
            "fetchInputRate",
            "fetchLgStatus",
            "fetchLimiterMeter",
            "fetchOutputPin",
            "fetchSpdifRxChannelStatus",
            "fetchSpdifRxPin",
            "fetchSpdifRxStatus",
            "fetchStat",
            "fetchStatus",
            "fetchSubharmHeadroom",
            "fetchSubharmMeter",
            "fetchUpmixStatus",
            "filtersExport",
            "filtersInspect",
            "getChannelName",
            "getPresetDirectory",
            "getPresetName",
            "getState",
            "inaccessibleDevices",
            "isConnected",
            "maxDelayMs",
            "monitorClear",
            "monitorRead",
            // -- WP CG23 -- The tap and the generation: local state, no device I/O.
            "notifyTapSet",
            "notifyTapDrain",
            "connectionGeneration",
            "openFd",
            "pollHotplug",
            "preampMaxDb",
            "presetChanges",
            "presetDirty",
            "processNotifications",
            "refreshParams",
            "rtaGetCaps",
            "rtaGetSnapshot",
            "rtaSetListener",
            "rtaSetRequest",
            "rtaStart",
            "scanDevices",
            "selectDevice",
            "setHotplugCallback",
            "setNotifyListener",
            "siggenFetchCaps",
            "siggenGetConfig",
            "siggenGetStatus",
            // -- WP AN1 --
            // Local files and the firmware-update path (as enterBootloader),
            // reads, connection management, the analyser's request.
            "autoeqDatabaseInfo",
            "autoeqImport",
            "autoeqUpdate",
            "autoeqResetToBuiltin",
            "autoeqRebuildWarning",
            "autoeqRebuildStart",
            "quantizeGain",
            "quantizeDelay",
            "openDemo",
            "extStatus",
            "fetchCoreVreg",
            // -- WP FWX --
            "fetchIdentify",
            "fetchResetInfo",
            "resetInfoAtConnect",
            "hasClipEvents",
            // -- WP SW1 -- the audio link's reads, and the demo with it (as openDemo)
            "alinkInfo",
            "fetchAlinkInfo",
            "fetchAlinkStatus",
            "fetchAlinkSlots",
            "fetchAlinkPhase",
            "fetchAlinkFrameCounter",
            "fetchApplyAtFrameStatus",
            "openDemoAlink",
            // -- WP AUSB1 -- the USB-direct block's reads, and the demo with it
            "usbDirectSupported",
            "fetchAlinkUsb",
            "fetchAlinkUsbStatus",
            "fetchAlinkUsbDiag",
            "fetchFrameCounterTimeBase",
            "openDemoUsbDirect",
            // -- WP CS1 -- the USB error counters (0xB2), a read
            "fetchUsbErrorStats",
            "uf2Inspect",
            "firmwareVersionMatches",
            "bootloaderDrives",
            "firmwareGuardBoard",
            "firmwareGuardBoardInBootloader",
            "firmwareGuardCheck",
            // -- WP SECFWV -- a hash against the bundled-firmware manifest, stateless
            "verifyBundled",
            "getInputSourceName",
            "getDeviceName",
            "fetchOutputLatency",
            "fetchOutputLatencyOptions",
            "fetchClockDiag",
            "linkRetryNow",
            "linkSetUrl",
            // -- WP LKUI1 -- the hub's audio link installation, a read
            "alinkSystemView",
            "rawGet",
            "rtaGetOwnership",
            "rtaTakeControl",
            // Review w1d (CG10, CG11, CG12, CG13, CG14): reads, the flow's
            // local state (open, close, keep and the view send nothing; the
            // poll only continues a run that the guarded start began) and
            // stateless text.
            "fetchRingHealth",
            "fetchOutputFills",
            "fetchStatsTick", // WP CG21: one read
            "coreVregCalView",
            "coreVregCalPoll",
            "coreVregCalOpen",
            "coreVregCalClose",
            "coreVregCalKeep",
            "linkPhaseText",
            "linkPhaseTexts",
            "onboardingWhatsNewJson",
            "onboardingGettingStartedJson",
            // WP CG15: stateless text.
            "aboutJson",
            // -- WP CG34 -- stateless: the filter types' names.
            "filterTypeNames",
            "filterTypeName",
            // -- BUG1 -- reads of the mirror, and stateless: output labels and cards.
            "outputLabels",
            "outputCards",
            "outputLabelsFor",
            "outputCardsFor",
            // -- WP CG24 -- The device lock across a flow: no device write
            // (the firmware-update path, as enterBootloader; the writes
            // inside it are guarded one by one).
            "deviceLockAcquire",
            "deviceLockRelease",
            // -- WP P0M -- the meters' ballistics (local) and the event history
            // (local files); fetchStatus's own clip clear is as clearClips.
            "meterBallistics",
            "setMeterBallistics",
            "resetMeterMax",
            "eventLogSetDir",
            "eventLogRead",
            "eventLogExport",
            "eventLogDevices",
            // -- WP P0W -- the change history's reads
            "canUndo",
            "canRedo",
            "history",
            // -- WP QTGAP1 -- the drag mark: how later writes join undo
            // steps; it sends nothing to the device.
            "setDragging",
            // -- WP P1UG -- the undo group marks: how later writes join undo
            // steps; they send nothing to the device.
            "beginGroup",
            "endGroup",
            // -- WP P0E -- the diagnostics bundle: device reads (the Stats
            // window's) and a local file; nothing is written to the device.
            "diagnosticsSave",
            "diagnosticsFileName",
            // -- WP P1NT -- stateless text: the notice table.
            "noticeFor",
            "noticeForKey",
            "noticeTexts",
            // -- WP ROMODE -- the core's client read-only mode: a switch that
            // refuses writes in the core (it sends nothing), its reading, and
            // the notice in that context.
            "setClientReadOnly",
            "clientReadOnly",
            "clientReadOnlyRefusals",
            "noticeForError",
            // -- WP P1EFIX -- stateless: the limits editor's range table
            "limitsFieldRanges",
            // -- WP P1PR -- the warning list, from the mirror: a read
            "outputsWithoutLimiter",
            // -- WP P1TK -- confirmation tokens: local state, nothing sent; and
            // the firmware-update path's confirmed forms (pass-through, as the
            // bootloader and drive writes always were: an old firmware must stay updatable)
            "confirmToken",
            "enterBootloaderConfirmed",
            "flashConfirmed",
            // -- WP P1LM -- the profile's files and status: the app's files,
            // nothing sent
            "limitsSetDir",
            "limitsProfileDocument",
            "limitsClear",
            "limitsStatus",
            // -- WP P1LK -- the mode manifest and the installer's PIN and lock:
            // stateless text and this console's own state and files; nothing
            // is sent to the device
            "modeManifest",
            "modeResolve",
            "modeSetPinned",
            "modePinned",
            "installerLockState",
            "installerUnlock",
            "installerLock",
            "installerPinSet",
            "installerPinClear",
            // -- WP P1DKFIX -- the hub's lock, read (`limits.get`,
            // `mode.pinned`): hub messages, nothing written to the device
            "installerLockRefresh",
            // -- WP HUBADMIN -- the hub's admin API: which copy the page edits
            // (no I/O), reads, and hub messages that write nothing to the
            // device (clearing a profile changes nothing on it; the PIN and
            // the pinned modes are the hub's); the hub's role check gates them
            "installerTarget",
            "hubLimitsGet",
            "hubLimitsClear",
            "hubPinSet",
            "hubPinClear",
            "hubModePin",
            "hubAdminEvents",
            "setHubAdminListener",
            // -- WP CG-SWS-a -- stateless tables and quantizers, and a read of
            // the mirror's image bytes: nothing is sent to the device
            "imageExtras",
            "quantizeMasterVolume",
            "curveSections",
            "knownFeatureNames",
            "linkParseTxt",
            "platformShape",
            "vregCodeMv",
            "vregCodeLabel",
            "defaultChannelNames",
            "enumValueLabels",
            "bulkImageExtras",
            // -- WP CLIP1 -- a copy reads the mirror; a clip's document is local
            "copyChannel",
            "channelClipToJson",
            "channelClipFromJson",
            // -- WP P2CC1 -- the hub's own families: reads, the watch and the
            // events, and the hub's snapshot store (the hub's files, as the
            // local store is the app's; nothing written to the device; the
            // hub's role check and the core's read-only mode gate them)
            "hubCaps",
            "hubInventory",
            "hubHealth",
            "hubStats",
            "hubEventsRead",
            "hubEventsWatch",
            "hubServerEvents",
            "setHubServerListener",
            "hubSnapshotsList",
            "hubSnapshotsGet",
            "hubSnapshotsPut",
            "hubSnapshotsRename",
            "hubSnapshotsNote",
            "hubSnapshotsDelete",
            "hubSnapshotsSave",
            "hubSnapshotsCopyHere",
            "snapshotListMerged",
            // -- WP P2CC2 -- the hub's admin families: reads, the notice, and
            // the hub's own settings (alarms, schedules, notifications out,
            // MQTT, API keys); nothing written to a device (the hub's role
            // check and the core's read-only mode gate them)
            "hubAlarms",
            "hubAlarmAck",
            "hubAlarmRulesGet",
            "hubAlarmRulesSet",
            "hubAlarmNotice",
            "hubSchedules",
            "hubScheduleSet",
            "hubScheduleDelete",
            "hubOutboundGet",
            "hubOutboundSet",
            "hubOutboundTest",
            "hubMqttGet",
            "hubMqttSet",
            "hubMqttStatus",
            "hubApiKeys",
            "hubApiKeyMint",
            "hubApiKeyRevoke",
            // -- WP P3PB -- the PICOBOOT writer: a board already in BOOTSEL, not
            // the device (pass-through as flashConfirmed; the core takes a
            // firmware_flash token and the guard first; Android has no bus, so
            // the listing is empty and the flash answers Unsupported)
            "picobootDevices",
            "picobootFlashConfirmed",
            // -- WP P3CC -- the hub's fleet families: reads, the step notice,
            // groups and the image store (the hub's own data), and the run's
            // confirm/cancel and the rollout's cancel (as applyConfirm and
            // applyRevert: settling never needs write access)
            "hubGroups",
            "hubGroupSet",
            "hubGroupDelete",
            "hubGroupParamGet",
            "hubGroupApplyConfirm",
            "hubGroupApplyCancel",
            "hubFirmwareImages",
            "hubFirmwarePut",
            "hubFirmwareDelete",
            "hubRollouts",
            "hubRolloutCancel",
            "hubRolloutStepNotice",
            // -- WP P5DOCK -- the measurement documents: the app's files and
            // stateless parsers; nothing is sent to the device
            "measureSetDir",
            "measureDocs",
            "measureDocGet",
            "measureDocPut",
            "measureDocRename",
            "measureDocNote",
            "measureDocDelete",
            "measurementExportRew",
            "measureDocInfo",
            "micImport",
            "targetImport",
            "targetBuiltins",
            // -- WP P5SES -- the measurement's reads, its capture, its tick (the
            // started session's own steps), its stop and its aborts (as
            // applyRevert: stopping never needs write access), its result
            "measureSetListener",
            "measurePreflight",
            "measureCapturePush",
            "measureTick",
            "measureNextPosition",
            "measureStop",
            "measureState",
            "measureResult",
            "measureClear",
            // -- WP P5LIVE -- the live view's reads, its tick (its started
            // runs), its previews (nothing written), its stop (as
            // measureStop), the RTA's hold and averaging
            "measureLiveTick",
            "measureLiveFrame",
            "measureLiveState",
            "measureLiveStop",
            "measureLivePredict",
            "measureLiveCutPeak",
            "measureLiveFitRange",
            "measureLiveDiscard",
            "measureLiveHold",
            "measureLiveAveraging",
            // -- WP P5HUB -- the hub's measurement documents: the hub's own data
            // (as hubSnapshots*), nothing sent to a device; the hub's role check
            // and the core's read-only mode gate the writes
            "hubMeasureDocs",
            "hubMeasureDocGet",
            "hubMeasureDocPut",
            "hubMeasureDocRename",
            "hubMeasureDocNote",
            "hubMeasureDocDelete",
            "measureDocsMerged",
            // -- WP P5DEMO -- the demo's simulated microphone: a client-side
            // microphone on the demo device only; nothing is sent beyond reads
            "measureSimSet",
            "measureSimInfo",
            // -- WP P5TUNE -- the layout and the proposal compute locally (reads);
            // the tick drives runs a guarded call started and the guard's timer
            // (as measureLiveTick, applyPoll); Keep and Revert end the guard as
            // applyConfirm and applyRevert do; the stop as measureStop
            "measureLayoutProposal",
            "measureTunePropose",
            "measureTuneProposal",
            "measureTuneTick",
            "measureTuneState",
            "measureTuneKeep",
            "measureTuneRevert",
            "measureTuneStop",
            // -- WP ATCORE -- the system measurement's unit check and preflight
            // (reads), its capture, its tick (the started session's own steps),
            // its state, stop, result and proposal; Keep and Revert end an apply
            // as measureTuneKeep and measureTuneRevert do
            "measureSystemUnitCheck",
            "measureSystemPreflight",
            "measureSystemCapturePush",
            "measureSystemTick",
            "measureSystemState",
            "measureSystemStop",
            "measureSystemResult",
            "measureSystemProposal",
            "measureSystemApplied",
            "measureSystemKeep",
            "measureSystemRevert",
            // -- WP PGCORE -- a raw paged GET (every paged GET reads) and the
            // connection's protocol generation (no traffic)
            "rawGetId",
            "deviceProtocol",
            // -- WP P5CURV -- the passive microphone RTA never touches the device
            // (no generator, no lock, no transfer); a document's curves compute locally
            "measureRtaPassiveStart",
            "measureRtaPassivePush",
            "measureRtaPassiveFrame",
            "measureRtaPassiveState",
            "measureRtaPassiveHold",
            "measureRtaPassiveAveraging",
            "measureRtaPassiveStop",
            "measureDocCurves",
            // -- WP PGFWX -- the update target (reads, before the bootloader
            // entry) and the update check (stateless, on the image)
            "firmwareTarget",
            "firmwareUpdateCheck",
            // -- review PG-D2 -- the write gate's verdict: reads the core's
            // memory (the target it kept), sends nothing
            "firmwareWriteCheck",
            // -- WP ROLE0 -- the speaker layout: the app's own files (as
            // limitsSetDir) and reads; nothing is sent to the device
            "speakerLayoutSetDir",
            "speakerLayoutSet",
            "speakerLayout",
            "speakerLayoutPropose",
            // -- WP INST0 -- the Installation setup: the view reads the hub
            // (system.get, the inventory); the draft calls compute locally
            "installationView",
            "installationDraftEdit",
            "installationValidate",
            "installationDiff",
            // -- WP FEAT2 -- what setting a room's or zone's volume would do: reads
            // (system.get, group.param.get)
            "systemVolumePlan",
            // -- WP ROLL1 -- the roll-off search's tick (its started segments, as
            // measureTick), its state, its stop (as measureStop) and its clear
            "measureRolloffTick",
            "measureRolloffState",
            "measureRolloffStop",
            "measureRolloffClear"
        )

        /** Control-surface operations that only read the device (`cs.rs`, `cs_op`). */
        private val CS_READ_OPS = setOf("refresh", "refreshStatus", "displayStatus", "auxRead", "learnRead")

        private val OP = Regex("\"op\"\\s*:\\s*\"(\\w+)\"")

        /** True only for a JSON op whose single "op" field names a read. */
        fun isCsRead(opJson: String): Boolean {
            val ops = OP.findAll(opJson).map { it.groupValues[1] }.toList()
            return ops.size == 1 && ops[0] in CS_READ_OPS
        }
    }
}
