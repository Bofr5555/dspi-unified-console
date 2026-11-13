import XCTest
import dspi_core
import dspi_constants
@testable import DSPi_Console

/// WP SW12: the control surfaces through the core (`csFetch`, `csSnapshot`,
/// `csApply`; Commands.swift "Control Surfaces"), on the core's demo device,
/// which carries a configuration of nearly everything (bindings, an IR
/// receiver with a learned key, groups, a macro, a display, aux outputs).
/// The edits go through the view model as the views make them. The core
/// records every operation under its `cs.*` path, and Undo steps over them
/// (the registry calls them deferred; the page's own Revert is the way back).
final class CoreDomainSW12Tests: XCTestCase, CoreDomainTesting {

    /// `body` off main, as the views call the setters that wait for a status.
    private func offMain<T>(_ body: () -> T) -> T { DispatchQueue.global().sync(execute: body) }

    /// Every request this domain has on the wire, SET and GET.
    private static let domainRequests: Set<UInt8> = [
        REQ_SET_CS_BINDING, REQ_GET_CS_BINDING, REQ_GET_CS_CAPS, REQ_GET_CS_STATUS, REQ_SET_CS_NAME,
        REQ_GET_CS_NAME, REQ_SET_CS_IR_CMD, REQ_GET_CS_IR_CMD, REQ_CS_IR_LEARN, REQ_CS_SAVE, REQ_CS_REVERT,
        REQ_SET_CS_GROUP, REQ_GET_CS_GROUP, REQ_SET_CS_MACRO, REQ_GET_CS_MACRO, REQ_SET_CS_MACRO_STEP,
        REQ_CS_MACRO_FIRE, REQ_GET_CS_EXT_STATUS, REQ_SET_CS_DISPLAY_CFG, REQ_GET_CS_DISPLAY_CFG,
        REQ_SET_CS_DISPLAY_PAGE, REQ_GET_CS_DISPLAY_PAGE, REQ_GET_CS_DISPLAY_STATUS, REQ_SET_CS_AUX_STATE,
        REQ_GET_CS_AUX_STATE, REQ_SET_CS_AUX_LEVEL, REQ_GET_CS_AUX_LEVEL,
    ]

    private func domainRaw(_ t: CoreBackedTransport) -> [String] {
        t.takeRawOpcodesForTest().filter { Self.domainRequests.contains($0) }.map { String(format: "0x%02X", $0) }
    }

    /// The demo device with its control surfaces read (the connect-time read
    /// fetches them).
    private func csVM() -> (DspiCore, CoreBackedTransport, DSPViewModel) {
        let (core, t, vm) = demoVM()
        XCTAssertTrue(spinUntil(10) { vm.controlSurfacesSupported && vm.csCaps.maxBindings > 0 },
                      "the connect-time read fetches the control surfaces through the core")
        return (core, t, vm)
    }

    /// The first configured binding of `type`.
    private func slot(_ vm: DSPViewModel, type: Int) -> Int? {
        vm.csBindings.indices.first { Int(vm.csBindings[$0].type) == type }
    }

    // MARK: - The snapshot

    /// The core's snapshot fills the Swift models the tab is built from.
    func testSW12TheSnapshotFillsTheModels() throws {
        let (core, t, vm) = csVM()
        let caps = vm.csCaps
        XCTAssertEqual(Int(caps.typeCount), caps.types.count)
        XCTAssertGreaterThan(caps.nounCount, 0)
        XCTAssertEqual(vm.csNounDescs.count, Int(caps.nounCount), "one descriptor per noun")
        XCTAssertTrue(vm.csNounDescs.contains { $0.actions != 0 }, "the nouns carry their actions")
        XCTAssertTrue(vm.csBindings.contains { $0.isConfigured }, "the demo's bindings")
        XCTAssertNotEqual(vm.csStatus.activeMask, 0, "live bindings")
        XCTAssertTrue(vm.csIrSupported)
        XCTAssertTrue(vm.csIrCommands.contains { $0.isConfigured && $0.code != 0 }, "the learned key")
        XCTAssertTrue(vm.csGroupsSupported && vm.csMacrosSupported && vm.csDisplaySupported)
        XCTAssertTrue(vm.csGroups.contains { $0.isConfigured }, "a group")
        XCTAssertTrue(vm.csMacros.contains { $0.isConfigured }, "a macro")
        XCTAssertGreaterThan(vm.csDisplayMaxPages, 0)
        XCTAssertTrue(vm.csDisplayPages.contains { $0.isActive }, "display pages")
        XCTAssertFalse(vm.csDirty, "the demo starts clean")

        // Field by field against the core's own JSON.
        let json = t.coreQueue.sync { core.csSnapshot() }
        let s = CsCoreSnapshot(json)
        XCTAssertTrue(s.supported)
        XCTAssertEqual(s.caps, vm.csCaps)
        XCTAssertEqual(s.bindings, vm.csBindings)
        XCTAssertEqual(s.names, vm.csNames)
        XCTAssertEqual(s.ir, vm.csIrCommands)
        XCTAssertEqual(s.groups, vm.csGroups)
        XCTAssertEqual(s.macros, vm.csMacros)
        XCTAssertEqual(s.displayCfg, vm.csDisplayCfg)
        XCTAssertEqual(s.displayPages, vm.csDisplayPages)
        XCTAssertEqual(s.status, vm.csStatus)
        // A binding goes back into the same JSON it came from.
        let b = try XCTUnwrap(vm.csBindings.first { $0.isConfigured })
        let back = CsCoreSnapshot(CsCoreJSON.text(["supported": true, "bindings": [CsCoreJSON.binding(b)]]))
        XCTAssertEqual(back.bindings[0], b)
        XCTAssertFalse(CsCoreSnapshot(#"{"supported": false}"#).supported)
        XCTAssertFalse(CsCoreSnapshot("not json").supported)
    }

    // MARK: - Edits, Save and Revert

    /// A rename and a binding edit apply through the core, mark the page
    /// unsaved, and Revert puts both back; Save keeps an edit and clears it.
    func testSW12EditsRevertAndSave() throws {
        let (core, t, vm) = csVM()
        let s = try XCTUnwrap(slot(vm, type: CS_TYPE_BUTTON), "the demo has a button")
        let name = vm.csNames[s]
        let binding = vm.csBindings[s]

        XCTAssertEqual(offMain { vm.setCsName(slot: s, name: "SW12 mute") }, PIN_CONFIG_SUCCESS)
        var edited = binding
        edited.flags ^= CS_FLAG_WRAP
        let status = offMain { vm.setCsBinding(slot: s, binding: edited) }
        XCTAssertTrue(spinUntil(5) { vm.csNames[s] == "SW12 mute" }, "the new name is shown")
        XCTAssertTrue(spinUntil(5) { vm.csDirty }, "unsaved")
        if status == PIN_CONFIG_SUCCESS {
            XCTAssertTrue(spinUntil(5) { vm.csBindings[s] == edited })
        } else {
            XCTAssertEqual(vm.csBindings[s], binding, "a refused edit is not shown")
        }
        XCTAssertEqual(entries(core, t, "cs.name").count, 1, "journalled")

        XCTAssertEqual(offMain { vm.csRevert() }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csNames[s] == name && vm.csBindings[s] == binding }, "reverted")
        XCTAssertTrue(spinUntil(5) { !vm.csDirty }, "clean after the revert")

        XCTAssertEqual(offMain { vm.setCsName(slot: s, name: "SW12 kept") }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csDirty })
        XCTAssertEqual(offMain { vm.csSave() }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { !vm.csDirty }, "clean after the save")
        XCTAssertEqual(vm.csNames[s], "SW12 kept")
        // A fresh read of the device finds it.
        offMain { vm.fetchControlSurfaces() }
        settle()
        XCTAssertEqual(vm.csNames[s], "SW12 kept")
        XCTAssertFalse(vm.csDirty)
    }

    /// The device's refusal comes back as its status byte, and the shown
    /// binding stays the device's.
    func testSW12ARefusedBindingReturnsTheDevicesStatus() throws {
        let (_, _, vm) = csVM()
        let s = try XCTUnwrap(slot(vm, type: CS_TYPE_BUTTON))
        let binding = vm.csBindings[s]
        var bad = binding
        bad.gpio0 = 30   // no such GPIO
        XCTAssertEqual(offMain { vm.setCsBinding(slot: s, binding: bad) }, PIN_CONFIG_INVALID_PIN)
        settle()
        XCTAssertEqual(vm.csBindings[s], binding)
        // A name past 31 bytes is cut on a character, as the firmware keeps it.
        XCTAssertEqual(CsCoreJSON.name(String(repeating: "é", count: 20)).utf8.count, 30)
    }

    /// A busy device (held in a revert) settles a write busy.
    func testSW12ABusyDeviceSettlesBusy() throws {
        let (core, t, vm) = csVM()
        let s = try XCTUnwrap(slot(vm, type: CS_TYPE_BUTTON))
        try t.coreQueue.sync { try core.csDemoBusy(on: true) }
        defer { try? t.coreQueue.sync { try core.csDemoBusy(on: false) } }
        XCTAssertEqual(offMain { vm.setCsName(slot: s, name: "SW12 busy") }, CS_STATUS_BUSY)
        XCTAssertNotEqual(offMain { vm.csSave() }, PIN_CONFIG_SUCCESS, "a save is refused meanwhile")
    }

    // MARK: - Groups, macros, display, aux

    func testSW12GroupsMacrosAndTheDisplay() throws {
        let (_, _, vm) = csVM()
        var group = vm.csGroups[1]
        group.name = "SW12 rears"
        if !group.isConfigured { group.targetKind = CS_TARGET_OUTPUT_CH; group.memberMask = 0b1100 }
        XCTAssertEqual(offMain { vm.setCsGroup(1, group: group) }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csGroups[1].name == "SW12 rears" })

        let m = try XCTUnwrap(vm.csMacros.indices.first { vm.csMacros[$0].isConfigured })
        var macro = vm.csMacros[m]
        macro.name = "SW12 film"
        XCTAssertEqual(offMain { vm.setCsMacro(m, macro: macro) }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csMacros[m].name == "SW12 film" })
        XCTAssertEqual(vm.csMacros[m].activeSteps, macro.activeSteps, "the steps kept")
        // The split setters write the whole macro through the core.
        XCTAssertEqual(offMain { vm.setCsMacroHeader(m, macro: CsMacro(name: "SW12 header", stepCount: macro.stepCount)) },
                       PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csMacros[m].name == "SW12 header" })
        XCTAssertEqual(vm.csMacros[m].activeSteps, macro.activeSteps)

        XCTAssertEqual(offMain { vm.csMacroFire(UInt16(m)) }, PIN_CONFIG_SUCCESS)
        XCTAssertNotNil(offMain { vm.fetchCsExtStatus() }, "the sequencer's state")
        _ = offMain { vm.csMacroFire(CS_MACRO_FIRE_CANCEL) }
        XCTAssertEqual(offMain { vm.csMacroFire(200) }, CS_STATUS_INVALID_MACRO, "no such macro")

        var cfg = vm.csDisplayCfg
        cfg.dwell = cfg.dwell == 40 ? 50 : 40
        XCTAssertEqual(offMain { vm.setCsDisplayCfg(cfg) }, PIN_CONFIG_SUCCESS)
        XCTAssertTrue(spinUntil(5) { vm.csDisplayCfg.dwell == cfg.dwell && vm.csDisplayLastStatus == PIN_CONFIG_SUCCESS })
        var page = vm.csDisplayPages[3]
        page.flags ^= CS_DPAGE_ACTIVE
        if page.isActive, page.noun == 0 { page.noun = vm.csDisplayPages.first { $0.isActive }?.noun ?? 0 }
        let pageStatus = offMain { vm.setCsDisplayPage(3, page: page) }
        if pageStatus == PIN_CONFIG_SUCCESS { XCTAssertTrue(spinUntil(5) { vm.csDisplayPages[3] == page }) }
        XCTAssertNotNil(offMain { vm.fetchCsDisplayStatus() })
        XCTAssertEqual(offMain { vm.fetchCsDisplayCfg() }, vm.csDisplayMaxPages)
    }

    /// An aux output's state and a drag of its level go out on the core's
    /// queue; the device holds the last level.
    func testSW12AuxOutputsThroughTheCore() throws {
        let (core, t, vm) = csVM()
        XCTAssertTrue(vm.csAuxSupported, "the demo's caps have aux outputs")
        let out = try XCTUnwrap(vm.csBindings.indices.first {
            Int(vm.csBindings[$0].type) == CS_TYPE_AUX_OUT && vm.csStatus.isSlotActive($0)
        }, "an aux output")
        let on = vm.csAuxState[out]
        vm.setCsAuxState(out, on: !on)
        flush(t)
        XCTAssertEqual(CsCoreSnapshot(t.coreQueue.sync { core.csSnapshot() }).auxState[out], !on, "on the device")

        let pwm = try XCTUnwrap(vm.csBindings.indices.first {
            Int(vm.csBindings[$0].type) == CS_TYPE_AUX_PWM && vm.csStatus.isSlotActive($0)
        }, "a dimmable output")
        for pct in stride(from: 10, through: 40, by: 5) { vm.setCsAuxLevel(pwm, percent: Float(pct)) }
        flush(t)
        settle()
        XCTAssertEqual(CsCoreSnapshot(t.coreQueue.sync { core.csSnapshot() }).auxLevel[pwm], 40 * 256)
        XCTAssertEqual(vm.csAuxLevel[pwm], 40 * 256, "the drag's last level stays shown")
        offMain { vm.fetchCsAuxValues() }
        settle()
        XCTAssertEqual(vm.csAuxLevel[pwm], 40 * 256)
        XCTAssertEqual(vm.csAuxState[out], !on)
    }

    // MARK: - IR learn (CG24)

    /// Arm, the remote's key, the result; and a cancelled learn.
    func testSW12IrLearnThroughTheCore() throws {
        let (core, t, vm) = csVM()
        XCTAssertEqual(offMain { vm.csIrLearnArm() }, .armed)
        let armed = try XCTUnwrap(offMain { vm.csIrLearnRead() })
        XCTAssertEqual(armed.state, CS_IR_LEARN_STATE_ARMED)
        XCTAssertTrue(try t.coreQueue.sync { try core.csDemoLearn(protocol: 1, code: 0x00FF_12ED) }, "armed")
        let done = try XCTUnwrap(offMain { vm.csIrLearnRead() })
        XCTAssertTrue(done.isDone)
        XCTAssertEqual(done.proto, 1)
        XCTAssertEqual(done.code, 0x00FF_12ED)
        vm.csIrLearnEnd()   // nothing held here: the core ended its window on the result

        XCTAssertEqual(offMain { vm.csIrLearnArm() }, .armed)
        offMain { vm.csIrLearnCancel() }
        let cancelled = try XCTUnwrap(offMain { vm.csIrLearnRead() })
        XCTAssertNotEqual(cancelled.state, CS_IR_LEARN_STATE_ARMED)
        XCTAssertFalse(try t.coreQueue.sync { try core.csDemoLearn(protocol: 1, code: 1) }, "nothing armed")
        XCTAssertFalse(CsCoreReply(#"{"ok": false, "error": "device not connected"}"#).isLocked)
        XCTAssertTrue(CsCoreReply(#"{"ok": false, "error": "another console is applying changes"}"#).isLocked)
    }

    // MARK: - Undo per the registry

    /// A control-surface edit is recorded and stepped over: Undo goes back to
    /// the volume change before it and says so; the page keeps the edit.
    func testSW12UndoStepsOverControlSurfaceEdits() throws {
        let (core, t, vm) = csVM()
        XCTAssertFalse(vm.undo.canUndo)
        let volume = vm.masterVolumeDB
        vm.setMasterVolume(volume - 6)
        XCTAssertTrue(spinUntil(5) { vm.undo.canUndo })
        let s = try XCTUnwrap(slot(vm, type: CS_TYPE_BUTTON))
        XCTAssertEqual(offMain { vm.setCsName(slot: s, name: "SW12 undo") }, PIN_CONFIG_SUCCESS)
        let e = try XCTUnwrap(entries(core, t, "cs.name").last)
        XCTAssertFalse(e.undoable, "the registry: every cs.* path is deferred")

        vm.undo.undo()
        XCTAssertTrue(spinUntil(5) { vm.masterVolumeDB == volume }, "the volume change is undone")
        XCTAssertNotNil(vm.undo.notice, "the step over is named")
        XCTAssertEqual(vm.csNames[s], "SW12 undo", "the control-surface edit stays")
    }

    // MARK: - No raw requests in this domain

    /// Every read and write of the domain, on the core's demo device: no
    /// request of the domain goes out raw (each is a core call), the journal
    /// holds no raw entry, and the writes are the core's `cs.*` entries.
    func testSW12EveryDomainCallGoesThroughTheCore() throws {
        let (core, t, vm) = csVM()
        _ = t.takeRawOpcodesForTest()   // the connect-time read
        let s = try XCTUnwrap(slot(vm, type: CS_TYPE_BUTTON))
        let binding = vm.csBindings[s]
        let ir = vm.csIrCommands[0]
        let group = vm.csGroups[0]
        let macro = vm.csMacros[0]
        let cfg = vm.csDisplayCfg
        let page = vm.csDisplayPages[0]

        offMain {
            vm.fetchControlSurfaces()
            vm.fetchCsBinding(slot: s)
            vm.fetchCsName(slot: s)
            vm.fetchCsIrCommand(sub: 0)
            vm.fetchCsStatus()
            vm.fetchCsAuxValues()
            vm.fetchCsGroup(0)
            vm.fetchCsMacro(0)
            _ = vm.fetchCsExtStatus()
            _ = vm.fetchCsDisplayCfg()
            vm.fetchCsDisplayPage(0)
            _ = vm.fetchCsDisplayStatus()
            _ = vm.setCsBinding(slot: s, binding: binding)
            _ = vm.setCsName(slot: s, name: "SW12 all")
            _ = vm.setCsIrCommand(sub: 0, command: ir)
            _ = vm.setCsGroup(0, group: group)
            _ = vm.setCsMacroStep(macro: 0, step: 0, value: macro.steps[0])
            _ = vm.setCsMacroHeader(0, macro: macro)
            _ = vm.setCsMacro(0, macro: macro)
            _ = vm.csMacroFire(0)
            _ = vm.csMacroFire(CS_MACRO_FIRE_CANCEL)
            _ = vm.setCsDisplayCfg(cfg)
            _ = vm.setCsDisplayPage(0, page: page)
            _ = vm.csIrLearnArm()
            _ = vm.csIrLearnRead()
            vm.csIrLearnCancel()
            vm.csIrLearnEnd()
            _ = vm.csSave()
            _ = vm.csRevert()
        }
        vm.queueDisplayCfg(cfg)
        vm.queueDisplayPage(0, page)
        let out = try XCTUnwrap(slot(vm, type: CS_TYPE_AUX_OUT))
        let pwm = try XCTUnwrap(slot(vm, type: CS_TYPE_AUX_PWM))
        vm.setCsAuxState(out, on: vm.csAuxState[out])
        vm.setCsAuxLevel(pwm, percent: 20)
        vm.displaySetQueue.sync {}   // the queued display writes have run
        flush(t)
        settle()

        XCTAssertEqual(domainRaw(t), [], "no control-surface request went out raw")
        let history = t.coreQueue.sync { core.history(n: 200) }
        XCTAssertFalse(history.contains { $0.path == "raw" }, "no raw entry in the journal")
        for path in ["cs.binding", "cs.name", "cs.ir", "cs.group", "cs.macro", "cs.macro.fire", "cs.display",
                     "cs.display.page", "cs.ir.learn", "cs.save", "cs.revert", "cs.aux", "cs.aux.level"] {
            XCTAssertFalse(entries(core, t, path).isEmpty, "\(path) through the core")
        }
    }
}
