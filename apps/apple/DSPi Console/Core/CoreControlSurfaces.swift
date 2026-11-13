import Foundation

// WP SW12: the control surfaces through the core (docs/SWIFT-SEAMS.md section 3,
// "SW12").  The core keeps the device's control-surface configuration
// (`csFetch`, `csSnapshot`) and performs every operation on it (`csApply`), in
// the JSON the Qt `CsController` and Android's editor speak (crates/dspi-core/
// src/cs.rs).  The Swift tab keeps its typed models (`CsBinding`, `CsGroup`, ...):
// this file turns them into the core's JSON and the core's snapshot back into
// them, so the views stay as they are.

/// The core's JSON for the Swift control-surface models: what `csApply`
/// takes for a record (cs.rs, `binding_from_json` and its siblings).
enum CsCoreJSON {
    static func binding(_ b: CsBinding) -> [String: Any] {
        ["type": b.type, "noun": b.noun, "action": b.action, "flags": b.flags,
         "gpio0": b.gpio0, "gpio1": b.gpio1, "event": b.event, "target": b.target,
         "index": b.index, "baseBright": b.baseBright, "value": b.value, "step": b.step,
         "rangeMin": b.rangeMin, "rangeMax": b.rangeMax, "onDelay": b.onDelay,
         "offDelay": b.offDelay, "extras": b.extras]
    }

    static func ir(_ c: IrCommand) -> [String: Any] {
        ["noun": c.noun, "action": c.action, "flags": c.flags, "target": c.target,
         "index": c.index, "protocol": c.proto, "value": c.value, "step": c.step, "code": c.code]
    }

    static func group(_ g: CsGroup) -> [String: Any] {
        ["targetKind": g.targetKind, "memberMask": g.memberMask, "name": g.name]
    }

    static func step(_ s: CsMacroStep) -> [String: Any] {
        ["noun": s.noun, "action": s.action, "flags": s.flags, "target": s.target,
         "index": s.index, "value": s.value, "step": s.step, "preDelay": s.preDelay]
    }

    /// A macro as the core writes it: its name and the steps it runs (the
    /// core clears the step slots past them).
    static func macro(_ m: CsMacro) -> [String: Any] {
        ["name": m.name, "steps": m.steps.prefix(min(Int(m.stepCount), CS_MAX_MACRO_STEPS)).map(step)]
    }

    static func displayConfig(_ c: CsDisplayCfg) -> [String: Any] {
        ["mode": c.mode, "homePage": c.homePage, "dwell": c.dwell, "overlayHold": c.overlayHold,
         "brightness": c.brightness, "flags": c.flags, "editTimeout": c.editTimeout]
    }

    static func page(_ p: CsDisplayPage) -> [String: Any] {
        ["noun": p.noun, "target": p.target, "index": p.index, "flags": p.flags]
    }

    /// A name as the firmware keeps it: at most 31 bytes, cut on a character.
    static func name(_ name: String) -> String {
        var out = ""
        var bytes = 0
        for ch in name {
            let n = String(ch).utf8.count
            if bytes + n > CS_NAME_LEN - 1 { break }
            out.append(ch)
            bytes += n
        }
        return out
    }

    /// An operation as text for `csApply`.  The values are numbers, strings,
    /// arrays and dictionaries only, so this cannot fail; "{}" if it did.
    static func text(_ op: [String: Any]) -> String {
        guard JSONSerialization.isValidJSONObject(op),
              let d = try? JSONSerialization.data(withJSONObject: op) else { return "{}" }
        return String(decoding: d, as: UTF8.self)
    }
}

/// One `csApply` reply: `{ok, status}` for a write, op fields for a read, or
/// `{ok: false, error}` (a malformed op, no device, read-only, transfer).
struct CsCoreReply {
    let ok: Bool
    let status: UInt8?
    let error: String?
    let fields: [String: Any]
    /// -- fixup sw12 N3 -- The snapshot read with it, when there is one.
    var snapshot: CsCoreSnapshot?

    init(_ text: String) {
        let v = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any] ?? [:]
        fields = v
        ok = (v["ok"] as? Bool) ?? false
        status = (v["status"] as? NSNumber).flatMap { UInt8(exactly: $0.intValue) }
        error = v["error"] as? String
    }

    /// The PIN_CONFIG_* / CS_STATUS_* byte the raw path returned: 0 applied,
    /// the device's code when it refused, 0xFF when nothing came back.
    var statusByte: UInt8 {
        if ok { return status ?? PIN_CONFIG_SUCCESS }
        return status ?? 0xFF
    }

    /// The core refused the device lock (`DspiError::Locked`, whose words
    /// the reply's `error` carries: crates/dspi-core/src/error.rs).
    var isLocked: Bool { !ok && status == nil && error == CsCoreReply.lockedText }
    static let lockedText = "another console is applying changes"

    // -- fixup sw12 F2 --
    /// `DspiError::ReadOnly` and `Denied` as the reply's `error` (error.rs).
    static let readOnlyText = "the connected firmware is read-only in this version"
    static let deniedText = "this console's role on the hub does not allow that"

    /// A failure the core reported without a device status (nothing reached
    /// the device, or nothing is known to have): the core's words, and
    /// whether the view model re-reads the device (read-only, locked or
    /// denied: the hub's and the firmware's refusals, as `writeRefused`).
    /// nil for success and for the device's own refusal (its status byte).
    var refusal: (message: String, resync: Bool)? {
        guard !ok, status == nil else { return nil }
        let text = error ?? RefusedWrite.transferMessage
        let resync = [CsCoreReply.lockedText, CsCoreReply.readOnlyText, CsCoreReply.deniedText].contains(text)
        return (text, resync)
    }
}

/// The core's control-surface snapshot (`csSnapshot()`), as the Swift models.
struct CsCoreSnapshot {
    var supported = false
    var caps = CsCapsHeader()
    var nouns: [CsNounDesc] = []
    var status = CsStatusPacket()
    var bindings: [CsBinding] = Array(repeating: CsBinding(), count: CS_MAX_BINDINGS)
    var names: [String] = Array(repeating: "", count: CS_MAX_BINDINGS)
    var ir: [IrCommand] = Array(repeating: IrCommand(), count: CS_MAX_IR_COMMANDS)
    var groups: [CsGroup] = Array(repeating: CsGroup(), count: CS_MAX_GROUPS)
    var macros: [CsMacro] = Array(repeating: CsMacro(), count: CS_MAX_MACROS)
    var ext = CsExtStatusPacket()
    var displayCfg = CsDisplayCfg()
    var displayPages: [CsDisplayPage] = Array(repeating: CsDisplayPage(), count: CS_MAX_DISPLAY_PAGES)
    var displayStatus = CsDisplayStatus()
    var displayMaxPages: UInt8 = 0
    var displayModelCount: UInt8 = 0
    var auxState: [Bool] = Array(repeating: false, count: CS_MAX_BINDINGS)
    var auxLevel: [UInt16] = Array(repeating: 0, count: CS_MAX_BINDINGS)

    init() {}

    /// The snapshot's JSON (cs.rs, `snapshot`); `{"supported": false}`, or
    /// anything unreadable, is an unsupported device.
    init(_ text: String) {
        guard let v = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any],
              (v["supported"] as? Bool) == true else { return }
        supported = true

        let types = Self.list(v["types"]).map {
            CsTypeDesc(actions: Self.u16($0["actions"]), pinCount: Self.u8($0["pins"]),
                       pinClass: Self.u8($0["pinClass"]))
        }
        nouns = Self.list(v["nouns"]).map {
            CsNounDesc(kind: Self.u8($0["kind"]), enumCount: Self.u8($0["enumCount"]),
                       actions: Self.u16($0["actions"]), minQ8: Self.i16($0["min"]),
                       maxQ8: Self.i16($0["max"]), unit: Self.u8($0["unit"]),
                       targetKind: Self.u8($0["targetKind"]), targetCount: Self.u8($0["targetCount"]),
                       dflags: ($0["deferred"] as? Bool) == true ? 1 : 0)
        }
        let maxBindings = Self.u8(v["maxBindings"])
        caps = CsCapsHeader(capsVersion: Self.u8(v["capsVersion"]), maxBindings: maxBindings,
                            typeCount: UInt8(clamping: types.count), nounCount: UInt8(clamping: nouns.count),
                            types: types, maxIrCommands: Self.u8(v["maxIr"]),
                            maxGroups: Self.u8(v["maxGroups"]), maxMacros: Self.u8(v["maxMacros"]),
                            maxMacroSteps: Self.u8(v["maxSteps"]))

        if let s = v["status"] as? [String: Any] {
            status = CsStatusPacket(
                lastStatus: Self.u8(s["lastStatus"]), lastSlot: Self.u8(s["lastSlot"]),
                maxBindings: maxBindings, dirty: (s["dirty"] as? Bool) ?? false,
                activeMask: Self.u16(s["activeMask"]),
                slotStatus: Self.fixed(Self.numbers(s["slotStatus"]).map(Self.u8), CS_MAX_BINDINGS, 0),
                irActiveMask: Self.u16(s["irActiveMask"]), irLearnState: Self.u8(s["learnState"]),
                irCmdStatus: Self.fixed(Self.numbers(s["irStatus"]).map(Self.u8), CS_MAX_IR_COMMANDS, 0))
        }

        bindings = Self.fixed(Self.list(v["bindings"]).map(Self.binding), CS_MAX_BINDINGS, CsBinding())
        names = Self.fixed((v["names"] as? [Any] ?? []).map { $0 as? String ?? "" }, CS_MAX_BINDINGS, "")
        ir = Self.fixed(Self.list(v["ir"]).map(Self.irCommand), CS_MAX_IR_COMMANDS, IrCommand())
        groups = Self.fixed(Self.list(v["groups"]).map {
            CsGroup(targetKind: Self.u8($0["targetKind"]), memberMask: Self.u32($0["memberMask"]),
                    name: $0["name"] as? String ?? "")
        }, CS_MAX_GROUPS, CsGroup())
        macros = Self.fixed(Self.list(v["macros"]).map { m -> CsMacro in
            let steps = Self.list(m["steps"]).prefix(CS_MAX_MACRO_STEPS).map(Self.step)
            return CsMacro(name: m["name"] as? String ?? "", stepCount: UInt8(steps.count),
                           steps: Self.fixed(Array(steps), CS_MAX_MACRO_STEPS, CsMacroStep()))
        }, CS_MAX_MACROS, CsMacro())

        let e = v["ext"] as? [String: Any] ?? [:]
        ext = CsExtStatusPacket(
            maxGroups: caps.maxGroups, maxMacros: caps.maxMacros, maxMacroSteps: caps.maxMacroSteps,
            macroRunning: e["macroRunning"] == nil ? CS_MACRO_NONE : Self.u8(e["macroRunning"]),
            macroStep: Self.u8(e["macroStep"]),
            groupStatus: Self.fixed(Self.numbers(e["groupStatus"]).map(Self.u8), CS_MAX_GROUPS, 0),
            macroStatus: Self.fixed(Self.numbers(e["macroStatus"]).map(Self.u8), CS_MAX_MACROS, 0))

        if let d = v["display"] as? [String: Any] {
            displayMaxPages = Self.u8(d["maxPages"])
            displayModelCount = Self.u8(d["modelCount"])
            let c = d["config"] as? [String: Any] ?? [:]
            displayCfg = CsDisplayCfg(mode: Self.u8(c["mode"]), homePage: Self.u8(c["homePage"]),
                                      dwell: Self.u16(c["dwell"]), overlayHold: Self.u16(c["overlayHold"]),
                                      brightness: Self.u8(c["brightness"]), flags: Self.u8(c["flags"]),
                                      editTimeout: Self.u16(c["editTimeout"]))
            displayPages = Self.fixed(Self.list(d["pages"]).map {
                CsDisplayPage(noun: Self.u8($0["noun"]), target: Self.u8($0["target"]),
                              index: Self.u8($0["index"]), flags: Self.u8($0["flags"]))
            }, CS_MAX_DISPLAY_PAGES, CsDisplayPage())
            let s = d["status"] as? [String: Any] ?? [:]
            displayStatus = CsDisplayStatus(initState: Self.u8(s["init"]), currentPage: Self.u8(s["page"]),
                                            flags: Self.u8(s["flags"]), model: Self.u8(s["model"]),
                                            nakCount: Self.u16(s["naks"]))
        }

        auxState = Self.fixed(Self.numbers(v["auxState"]).map { $0.intValue != 0 }, CS_MAX_BINDINGS, false)
        auxLevel = Self.fixed(Self.numbers(v["auxLevel"]).map { min(Self.u16($0), CS_AUX_LEVEL_MAX_Q8) },
                              CS_MAX_BINDINGS, 0)
    }

    // MARK: Decoding helpers (a missing or odd field reads as zero)

    private static func list(_ v: Any?) -> [[String: Any]] {
        (v as? [Any] ?? []).map { $0 as? [String: Any] ?? [:] }
    }

    private static func numbers(_ v: Any?) -> [NSNumber] {
        (v as? [Any] ?? []).map { $0 as? NSNumber ?? 0 }
    }

    private static func int(_ v: Any?) -> Int64 { (v as? NSNumber)?.int64Value ?? 0 }
    private static func u8(_ v: Any?) -> UInt8 { UInt8(truncatingIfNeeded: int(v)) }
    private static func u16(_ v: Any?) -> UInt16 { UInt16(truncatingIfNeeded: int(v)) }
    private static func i16(_ v: Any?) -> Int16 { Int16(truncatingIfNeeded: int(v)) }
    private static func u32(_ v: Any?) -> UInt32 { UInt32(truncatingIfNeeded: int(v)) }

    /// Exactly `n` entries, padded with `pad` (the Swift tables' fixed size).
    private static func fixed<T>(_ a: [T], _ n: Int, _ pad: T) -> [T] {
        var out = Array(a.prefix(n))
        while out.count < n { out.append(pad) }
        return out
    }

    private static func binding(_ b: [String: Any]) -> CsBinding {
        CsBinding(type: u8(b["type"]), noun: u8(b["noun"]), action: u8(b["action"]), flags: u8(b["flags"]),
                  gpio0: u8(b["gpio0"]), gpio1: u8(b["gpio1"]), event: u8(b["event"]),
                  target: u8(b["target"]), index: u8(b["index"]), baseBright: u8(b["baseBright"]),
                  value: i16(b["value"]), step: i16(b["step"]), rangeMin: i16(b["rangeMin"]),
                  rangeMax: i16(b["rangeMax"]), onDelay: u16(b["onDelay"]), offDelay: u16(b["offDelay"]),
                  extras: u8(b["extras"]))
    }

    private static func irCommand(_ c: [String: Any]) -> IrCommand {
        IrCommand(noun: u8(c["noun"]), action: u8(c["action"]), flags: u8(c["flags"]),
                  target: u8(c["target"]), index: u8(c["index"]), proto: u8(c["protocol"]),
                  value: i16(c["value"]), step: i16(c["step"]), code: u32(c["code"]))
    }

    private static func step(_ s: [String: Any]) -> CsMacroStep {
        CsMacroStep(noun: u8(s["noun"]), action: u8(s["action"]), flags: u8(s["flags"]),
                    target: u8(s["target"]), index: u8(s["index"]), value: i16(s["value"]),
                    step: i16(s["step"]), preDelay: u16(s["preDelay"]))
    }
}
