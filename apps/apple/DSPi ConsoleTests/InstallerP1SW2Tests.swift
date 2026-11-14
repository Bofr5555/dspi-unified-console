import XCTest
import Combine
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP P1SW2: the installer's tools through the core (Core/CoreInstaller.swift,
/// Installer/InstallerViews.swift), the tokens, the undo groups, the AutoEQ
/// preview and the crossfeed presets (P1XF), on the core's demo device.  In
/// the macOS and the iOS bundles.
final class InstallerP1SW2Tests: XCTestCase, CoreDomainTesting {

    private var folders: [URL] = []

    override func tearDown() {
        for f in folders { try? FileManager.default.removeItem(at: f) }
        folders = []
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        super.tearDown()
    }

    /// A fresh folder of the test's own (the VM's temporary directory).
    private func folder(_ name: String) -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("P1SW2-\(name)-\(UUID().uuidString)", isDirectory: true)
        folders.append(url)
        return url
    }

    /// The demo device, a view model on it and its installer wrapper, ready.
    private func setUpInstaller() -> (DspiCore, CoreBackedTransport, DSPViewModel, CoreInstaller) {
        let d = UserDefaults.standard
        for key in d.dictionaryRepresentation().keys where key.hasPrefix("uiMode.") { d.removeObject(forKey: key) }
        let (core, t, vm) = demoVM()
        let installer = CoreInstaller(vm: vm, folder: folder("limits"))
        XCTAssertTrue(spinUntil(10) { installer.available && !installer.busy && installer.choice != nil },
                      "the wrapper sees the core")
        return (core, t, vm, installer)
    }

    private func state(_ t: CoreBackedTransport) -> DspState { t.readState() }

    private func waitIdle(_ installer: CoreInstaller, _ seconds: TimeInterval = 10) -> Bool {
        spinUntil(seconds) { !installer.busy }
    }

    // MARK: - No PIN: nothing asks

    func testP1SW2WithNoPinNothingIsLockedAndTheModeIsThePlatformDefault() {
        let (_, _, _, installer) = setUpInstaller()
        XCTAssertFalse(installer.lock.pinSet)
        XCTAssertTrue(installer.installerOpen, "no PIN: the Installer pages are open")
        XCTAssertFalse(installer.lockedUser)
        #if os(macOS)
        XCTAssertEqual(installer.mode, "advanced", "the Mac opens in Advanced (the manifest's desktop default)")
        XCTAssertFalse(installer.simple)
        #else
        XCTAssertEqual(installer.mode, "simple", "iOS opens in Simple (the manifest's phone default)")
        XCTAssertTrue(installer.simple)
        #endif
        XCTAssertEqual(installer.choice?.reason, "default")
        XCTAssertTrue(installer.switchable)
        XCTAssertFalse(installer.installerMode)

        installer.choose("installer", everyDevice: false)
        XCTAssertTrue(spinUntil(5) { installer.mode == "installer" }, "no PIN: the Installer opens without a prompt")
        XCTAssertFalse(installer.unlockShown, "nothing asks for a PIN")
        XCTAssertTrue(installer.installerMode)
        XCTAssertEqual(UserDefaults.standard.string(forKey: CoreInstaller.modeKey(serial: FakeTransport.demoDevice.serial)),
                       "installer", "the choice is kept for this device")

        installer.choose("simple", everyDevice: true)
        XCTAssertTrue(spinUntil(5) { installer.mode == "simple" })
        XCTAssertEqual(UserDefaults.standard.string(forKey: CoreInstaller.modeKeyAll), "simple")
        XCTAssertNil(UserDefaults.standard.string(forKey: CoreInstaller.modeKey(serial: FakeTransport.demoDevice.serial)),
                     "every device: the device's own choice gives way")
    }

    // MARK: - The limits profile

    func testP1SW2TheRangesAreTheCoresAndANarrowedLimitHoldsAndRefuses() {
        let (_, t, vm, installer) = setUpInstaller()
        // The one range table (P1EFIX): no Swift numbers.
        let ranges = limitsFieldRanges()
        XCTAssertEqual(installer.ranges, ranges)
        for key in ["maxGainDb", "limiterCeilingDb", "highpassMinHz", "highpassMinSlopeDb", "delayMinMs", "delayMaxMs",
                    "maxPeqBoostDb", "masterVolumeMaxDb"] {
            XCTAssertNotNil(installer.range(key), "the core's table has \(key)")
        }
        let delay = try? XCTUnwrap(installer.range("delayMaxMs"))
        XCTAssertEqual(installer.clamp("delayMaxMs", 1_000_000), delay?.max)
        XCTAssertEqual(installer.clamp("delayMaxMs", -5), delay?.min)

        vm.setOutputGain(output: 0, db: 3)
        flush(t)
        XCTAssertTrue(spinUntil(5) { self.state(t).outputGainDb[0] == 3 })

        installer.setOutputLimit(0, "maxGainDb", -3)
        XCTAssertTrue(waitIdle(installer))
        XCTAssertTrue(spinUntil(10) { self.state(t).outputGainDb[0] == -3 }, "held at once: \(state(t).outputGainDb[0])")
        XCTAssertTrue(spinUntil(10) { vm.outputGainDB[0] == -3 }, "and shown")
        XCTAssertTrue(installer.notice?.text.contains("out.gain[0]") ?? false,
                      "what was held, in the core's words: \(installer.notice?.text ?? "none")")
        XCTAssertEqual(installer.draft.outputValue(0, "maxGainDb"), -3)
        XCTAssertTrue(spinUntil(5) { installer.status?.active == true }, "the profile is in force")

        // A write past it is refused before anything is sent, with the row.
        vm.setOutputGain(output: 0, db: 0)
        XCTAssertTrue(spinUntil(5) { DeviceLock.notice.message == CoreInstaller.line("limit_exceeded") },
                      "the core's notice row: \(DeviceLock.notice.message ?? "none")")
        // Nothing was sent (the screen's put-back is the app's resync, as for
        // every refused write: SW9/SW10's tests).
        flush(t)
        XCTAssertEqual(state(t).outputGainDb[0], -3, "nothing reached the device")

        installer.setText("note", "  P1SW2 note ")
        XCTAssertTrue(waitIdle(installer))
        XCTAssertTrue(spinUntil(5) { installer.draft.text("note") == "P1SW2 note" })

        installer.clearLimits()
        XCTAssertTrue(waitIdle(installer))
        XCTAssertTrue(spinUntil(5) { installer.draft.isEmpty && installer.status?.active == false })
        vm.setOutputGain(output: 0, db: 0)
        XCTAssertTrue(spinUntil(5) { self.state(t).outputGainDb[0] == 0 }, "no limits: the write passes")
    }

    // MARK: - The PIN, the lock and the mode

    func testP1SW2PinRecoveryCodeLockCoverUnlockAndWrongPins() throws {
        let (_, t, vm, installer) = setUpInstaller()
        installer.setPin("1234", again: "1235")
        XCTAssertEqual(installer.notice?.text, "The two PINs differ.")
        XCTAssertFalse(installer.lock.pinSet)

        installer.setPin("1234", again: "1234")
        XCTAssertTrue(spinUntil(10) { installer.lock.pinSet && installer.recoveryCode != nil })
        let code = try XCTUnwrap(installer.recoveryCode)
        XCTAssertNotNil(code.range(of: "^[A-Z0-9]{5}(-[A-Z0-9]{5}){3}$", options: .regularExpression),
                        "the recovery code, shown once: \(code)")
        XCTAssertTrue(installer.lock.unlocked, "set: unlocked for this session")
        XCTAssertFalse(installer.lockedUser, "a PIN alone locks nothing")
        XCTAssertEqual(installer.lock.heldBy, "console")

        installer.setCovered(true)
        XCTAssertTrue(spinUntil(5) { !installer.lock.covers.isEmpty })
        XCTAssertEqual(Set(installer.lock.covers), Set(installer.coverPages))
        XCTAssertFalse(installer.coverPages.contains("front"), "the locked user's front page stays open")

        installer.lockNow()
        XCTAssertTrue(spinUntil(10) { installer.lockedUser && installer.mode == "locked_user" }, "locked")
        XCTAssertFalse(installer.switchable)
        XCTAssertFalse(installer.installerMode)
        XCTAssertTrue(installer.simple, "the locked user sees Simple")
        XCTAssertEqual(installer.scopeLine, CoreInstaller.reason("installer_lock.scope"),
                       "the lock is on this console (OD-P1d)")

        // Only volume, mute and presets.
        vm.setOutputGain(output: 1, db: -2)
        let lockedRow = CoreInstaller.line(for: .InstallerLock(code: "locked", msg: ""))
        XCTAssertTrue(spinUntil(5) { DeviceLock.notice.message == lockedRow },
                      "refused with the lock's row: \(DeviceLock.notice.message ?? "none")")
        XCTAssertEqual(state(t).outputGainDb[1], 0)
        let v = state(t).masterVolumeDb
        vm.setMasterVolume(v - 1)
        XCTAssertTrue(spinUntil(5) { self.state(t).masterVolumeDb == v - 1 }, "volume passes")
        // A gated call while locked is refused too (a flash save).
        XCTAssertNotEqual(vm.savePreset(slot: 1, confirmed: .savePreset()), PRESET_OK)

        // Wrong, then the recovery code.
        installer.unlock("9999")
        XCTAssertTrue(spinUntil(10) { installer.notice?.text == CoreInstaller.line("installer_lock.wrong_pin") },
                      "wrong: \(installer.notice?.text ?? "none")")
        XCTAssertTrue(installer.lockedUser)
        var ok: Bool?
        installer.unlock(code.lowercased()) { ok = $0 }
        XCTAssertTrue(spinUntil(10) { ok != nil })
        XCTAssertEqual(ok, true)
        XCTAssertTrue(spinUntil(5) { !installer.lockedUser && installer.lock.recovered }, "the recovery code unlocks")

        // Locked again, then wrong PINs until the core makes us wait.
        installer.lockNow()
        XCTAssertTrue(spinUntil(10) { installer.lockedUser })
        for _ in 0..<5 {
            var done = false
            installer.unlock("0000") { _ in done = true }
            XCTAssertTrue(spinUntil(10) { done })
        }
        XCTAssertTrue(spinUntil(10) { installer.notice?.text.contains("Try again in") ?? false },
                      "the back-off, in the core's row and the time: \(installer.notice?.text ?? "none")")
        XCTAssertTrue(installer.notice?.text.hasPrefix(CoreInstaller.line("installer_lock.wait")) ?? false)
        XCTAssertTrue(installer.lock.retryAfterMs > 0)
        XCTAssertTrue(installer.lockedUser)
    }

    func testP1SW2TheInstallerModeAsksForThePinOnlyWhenOneIsSet() {
        let (_, _, _, installer) = setUpInstaller()
        installer.setPin("4321", again: "4321")
        XCTAssertTrue(spinUntil(10) { installer.lock.pinSet })
        installer.lockNow()   // nothing covered: not the locked user, the Installer needs the PIN
        XCTAssertTrue(spinUntil(10) { !installer.installerOpen })
        XCTAssertFalse(installer.lockedUser)

        installer.choose("installer", everyDevice: false)
        XCTAssertTrue(installer.unlockShown, "the one prompt")
        XCTAssertNotEqual(installer.mode, "installer", "the default meanwhile")
        installer.unlockShown = false
        var ok: Bool?
        installer.unlock("4321") { ok = $0 }
        XCTAssertTrue(spinUntil(10) { ok == true })
        XCTAssertTrue(spinUntil(10) { installer.mode == "installer" }, "the choice runs once unlocked")
        XCTAssertTrue(installer.unlockedInstaller, "the Installer line with Lock Now")

        installer.removePin()
        XCTAssertTrue(spinUntil(10) { !installer.lock.pinSet && installer.installerOpen })
        XCTAssertEqual(installer.notice?.text, "Installer PIN removed: nothing is locked.")
    }

    // MARK: - Tokens (P1TK)

    func testP1SW2TokensAreRequiredAndTheAppsGatedCallsCarryOne() {
        let (_, t, vm) = demoVM()
        // P1TZ: required by every core from the start (no switch); a save without a token is refused.
        if case .failure(let e) = t.call({ try $0.savePresetConfirmed(slot: 2, token: nil) }) {
            if case .ConfirmationRequired = e {} else { XCTFail("refused for the token: \(e)") }
        } else {
            XCTFail("a save without a token is refused")
        }
        XCTAssertEqual(vm.savePreset(slot: 2, confirmed: .savePreset()), PRESET_OK, "the app's save mints its token")
        XCTAssertEqual(vm.copyPreset(from: 2, to: 3, confirmed: .copyPreset()), PRESET_OK)
        XCTAssertEqual(vm.deletePreset(slot: 3, confirmed: .deletePreset()), PRESET_OK)
        XCTAssertTrue(vm.saveOutputConfig(confirmed: .saveOutputConfig()))
        XCTAssertEqual(vm.saveParams(confirmed: .saveParams()), FLASH_OK)
        XCTAssertEqual(vm.factoryReset(confirmed: .factoryReset()), FLASH_OK)
        // An import through the core takes one too.
        let doc = (try? t.call { try $0.configExport(name: "P1SW2", appVersion: "test", links: 0) }.get()) ?? ""
        let reply = t.writeAndWait { $0.configImportConfirmed(text: doc, options: 0, eqChannels: allEqChannels,
                                                                token: $0.confirmToken(op: .configImport)) }
        let text = (try? reply.get()) ?? ""
        let object = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any]
        XCTAssertEqual(object?["ok"] as? Bool, true, "imported with its token: \(text)")
        XCTAssertEqual(CoreInstaller.line(for: .ConfirmationRequired(code: "missing", msg: "")),
                       CoreInstaller.line("confirmation_required"), "the row for a missing token")
    }

    // MARK: - Undo groups (P1UG)

    func testP1SW2ClearAllIsOneUndoStep() {
        let (core, t, vm) = demoVM()
        let ch = vm.chOut1
        for b in 0..<3 {
            vm.setFilter(ch: ch, band: b, p: DSPi_Console.FilterParams(type: .peaking, freq: Float(500 * (b + 1)), q: 1, gain: 3))
        }
        flush(t)
        XCTAssertTrue(spinUntil(5) { (vm.channelData[ch] ?? []).prefix(3).allSatisfy { $0.type == .peaking } })
        vm.clearPEQBands(ch: ch)
        flush(t)
        XCTAssertTrue(spinUntil(5) { (vm.channelData[ch] ?? []).allSatisfy { $0.type == .flat } })
        let labelled = t.coreQueue.sync { core.history(n: 50) }.filter { $0.label == DSPViewModel.clearAllLabel }
        XCTAssertFalse(labelled.isEmpty, "the step carries the group's label")
        undoAndRedo(vm, undone: { (vm.channelData[ch] ?? []).prefix(3).allSatisfy { $0.type == .peaking && $0.gain == 3 } },
                    redone: { (vm.channelData[ch] ?? []).allSatisfy { $0.type == .flat } })
    }

    func testP1SW2ALinkedPairsPreampIsOneUndoStep() {
        let (_, t, vm) = demoVM()
        vm.linkedInputPairs = 0b1
        XCTAssertEqual(vm.linkedPartner(of: 0), 1)
        vm.setPreampChannel(channel: 0, db: -5)
        flush(t)
        XCTAssertTrue(spinUntil(5) { self.state(t).inputPreampDb.prefix(2) == [-5, -5] }, "both inputs")
        undoAndRedo(vm, undone: { vm.preampDB[0] == 0 && vm.preampDB[1] == 0 },
                    redone: { vm.preampDB[0] == -5 && vm.preampDB[1] == -5 })
    }

    func testP1SW2DirectRoutingIsOneUndoStep() {
        let (_, t, vm) = demoVM()
        // The outputs on first, so the routing's own step stands alone (the
        // enables are never undone and stay outside the group).
        let n = min(vm.numMatrixInputs, vm.numOutputChannels)
        if vm.outputEnabled[vm.pdmOutputIndex] { vm.setOutputEnable(output: vm.pdmOutputIndex, enabled: false) }
        for o in 0..<n where o != vm.pdmOutputIndex && !vm.outputEnabled[o] { vm.setOutputEnable(output: o, enabled: true) }
        flush(t)
        vm.setMatrixRoute(input: 0, output: 1, enabled: true, gain: -6, invert: false)
        flush(t)
        XCTAssertTrue(spinUntil(5) { vm.matrixRouting[0][1] })
        settle(1)
        vm.applyDirectRouting()
        flush(t)
        XCTAssertTrue(spinUntil(5) { !vm.matrixRouting[0][1] && vm.matrixRouting[0][0] }, "the diagonal")
        undoAndRedo(vm, undone: { vm.matrixRouting[0][1] && vm.matrixGain[0][1] == -6 },
                    redone: { !vm.matrixRouting[0][1] && vm.matrixRouting[0][0] })
    }

    func testP1SW2ProtectAllIsOneUndoAndTheLimitersStayOn() {
        let (_, t, vm, installer) = setUpInstaller()
        vm.setOutputGain(output: 0, db: 0)
        if vm.outputEnabled[0] == false { vm.setOutputEnable(output: 0, enabled: true) }
        vm.setLimiterEnabled(output: 0, false)
        flush(t)
        installer.refresh()
        XCTAssertTrue(waitIdle(installer))
        XCTAssertTrue(spinUntil(5) { installer.withoutLimiter.contains(0) },
                      "S2: an output above -10 dB with no limiter: \(installer.withoutLimiter)")
        installer.protectAllOutputs()
        XCTAssertTrue(spinUntil(10) { installer.withoutLimiter.isEmpty && !installer.busy })
        XCTAssertTrue(state(t).limiterEnabled[0])
        XCTAssertTrue(installer.notice?.text.contains("-1 dBFS") ?? false)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.undo.notice?.contains("The limiters stay on") ?? false },
                      "the core's note: \(vm.undo.notice ?? "none")")
    }

    // MARK: - AutoEQ preview (P1AQ)

    private let entry = HeadphoneEntry(id: "p1sw2/test", manufacturer: "P1SW2", model: "Test", source: "oratory1990",
                                       formFactor: "over-ear", preamp: -6,
                                       filters: [EmbeddedFilter(type: "peaking", freq: 100, q: 1, gain: 4),
                                                 EmbeddedFilter(type: "highShelf", freq: 8000, q: 0.7, gain: -3)])

    func testP1SW2AutoEQPreviewSendsNothingAndApplyIsOneStep() {
        let (_, t, vm) = demoVM()
        let before = state(t)
        let model = AutoEQPreviewModel(entry: entry, vm: vm)
        XCTAssertEqual(model.pairs.first?.isOutput, false, "inputs first")
        XCTAssertTrue(model.pairs.contains { $0.isOutput }, "then the outputs")
        XCTAssertTrue(spinUntil(10) { model.result != nil }, "previewed: \(model.error ?? "")")
        XCTAssertEqual(model.result?.levelTarget, "input_preamp")
        XCTAssertFalse(model.result?.channels.first?.curve.isEmpty ?? true, "the curve to draw")
        XCTAssertEqual(state(t), before, "a preview sends nothing")

        let out = model.pairs.first { $0.isOutput }
        model.pair = out
        XCTAssertTrue(spinUntil(10) { model.result?.levelTarget == "none" }, "outputs keep their gain by default")
        XCTAssertEqual(model.result?.suggestedTrimDb, -6)
        XCTAssertTrue(model.levelLine.hasPrefix("Suggested trim"))
        let gain = state(t).outputGainDb

        var applied: Bool?
        model.apply { applied = $0 }
        XCTAssertTrue(spinUntil(15) { applied != nil })
        XCTAssertEqual(applied, true)
        let l = Int(out?.left ?? 0)
        XCTAssertTrue(spinUntil(5) { vm.channelData[l]?.first?.type == .peaking }, "the profile is on the outputs")
        XCTAssertEqual(state(t).outputGainDb, gain, "their gain untouched")
        XCTAssertEqual(state(t).filters[0], before.filters[0], "nothing outside the pair")
        undoAndRedo(vm, undone: { vm.channelData[l]?.first?.type == .flat },
                    redone: { vm.channelData[l]?.first?.type == .peaking })
    }

    // MARK: - P1XF

    func testP1SW2ACrossfeedPresetShowsItsOwnValuesAndCustomComesBack() {
        let (_, t, vm) = demoVM()
        vm.setCrossfeedPreset(3)
        vm.setCrossfeedFreq(900)
        vm.setCrossfeedFeed(3)
        flush(t)
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.preset == 3 && vm.crossfeed.freq == 900 && vm.crossfeed.feed == 3 })
        settle(0.5)

        vm.setCrossfeedPreset(1)   // Chu Moy
        flush(t)
        settle(0.5)
        XCTAssertEqual(vm.crossfeed.preset, 1, "never flips to Custom")
        XCTAssertEqual(vm.crossfeed.freq, 700)
        XCTAssertEqual(vm.crossfeed.feed, 6)
        // The core's mirror, and after a whole re-read.
        XCTAssertEqual(state(t).crossfeedFreq, 700)
        XCTAssertEqual(state(t).crossfeedFeed, 6)
        DispatchQueue.global().async { vm.applyCoreReread(t) }   // as after a fetch
        settle(1)
        XCTAssertEqual(vm.crossfeed.preset, 1)
        XCTAssertEqual(vm.crossfeed.freq, 700, "the graph draws the preset, not the custom pair")
        XCTAssertEqual(vm.crossfeed.feed, 6)

        vm.setCrossfeedPreset(3)
        flush(t)
        XCTAssertTrue(spinUntil(5) { vm.crossfeed.preset == 3 && vm.crossfeed.freq == 900 && vm.crossfeed.feed == 3 },
                      "Custom brings the custom pair back: \(vm.crossfeed.freq) / \(vm.crossfeed.feed)")
    }

    // MARK: - The words

    func testP1SW2TheWordsAreTheCoresRows() {
        XCTAssertFalse(CoreInstaller.bannerLine.isEmpty)
        XCTAssertEqual(CoreInstaller.bannerLine, noticeForKey(key: "installer_lock.banner", detail: "").reason)
        XCTAssertFalse(CoreInstaller.reason("limits.scope").isEmpty)
        XCTAssertFalse(CoreInstaller.reason("installer_lock.hub").isEmpty)
        XCTAssertTrue(CoreInstaller.line("limit_exceeded").hasPrefix("Outside the limits"))
        XCTAssertEqual(CoreInstaller.retryLine(30_000), "Try again in 30 s.")
        XCTAssertEqual(CoreInstaller.retryLine(29_001), "Try again in 30 s.")
        XCTAssertEqual(CoreInstaller.retryLine(900_000), "Try again in 15 min.")
        let m = modeManifest()
        #if os(macOS)
        XCTAssertEqual(CoreInstaller.defaultMode(m), "advanced")
        #else
        XCTAssertEqual(CoreInstaller.defaultMode(m), "simple")
        #endif
        // A draft keeps fields it does not know and drops the serial.
        var draft = LimitsDraft(document: """
            {"schemaVersion":1,"meta":{"kind":"limits"},"limits":{"serial":"X","future":5,"outputs":[{"output":2,"maxGainDb":3}]}}
            """)
        XCTAssertEqual(draft.outputValue(2, "maxGainDb"), 3)
        draft.setOutput(2, "maxGainDb", nil)
        XCTAssertNil(draft.json["outputs"], "an empty output row goes")
        let doc = draft.document(name: "Limits profile")
        XCTAssertTrue(doc.contains("\"future\":5"))
        XCTAssertFalse(doc.contains("serial"))
    }
}
