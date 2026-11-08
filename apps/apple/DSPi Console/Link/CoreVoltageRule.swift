//
//  CoreVoltageRule.swift
//  DSPi Console
//
//  WP SWF2 (hw-mac F4): the owner's rule for raising the core voltage, as the
//  Swift hub applies it to its network sessions.  A port of the core's
//  `dspi_session::overvolt` (WP CG14), which the Rust hub applies the same way
//  (HB2, `Router::before_exec`).
//
//  The confirmation word is required exactly when the target is above
//  1200 mV and above the unit's current voltage, read from the unit at that
//  moment.  A unit whose voltage cannot be read requires it for any target
//  above 1200 mV.  A hub cannot type the word for anyone, so a request that
//  needs it is refused from every network session, whatever its role.  The
//  app's own session is exempt: the person at the machine is asked by the app.
//

import Foundation

/// One `EXT_CORE_VREG` write: the `{op, value}` payload.
enum CoreVregOp: Equatable {
    case set(UInt8)
    case trial(UInt8)
    case trialEnd
    case auto(Bool)
    case clear

    /// Decode an `EXT_CORE_VREG` OUT payload; nil for anything the firmware
    /// does not define, which the rule refuses unread.
    init?(payload: Data) {
        let b = [UInt8](payload)
        guard b.count == 2 else { return nil }
        switch (b[0], b[1]) {
        case (0, let v): self = .set(v)
        case (1, let v): self = .trial(v)
        case (2, _): self = .trialEnd
        case (3, let v) where v <= 1: self = .auto(v == 1)
        case (4, _): self = .clear
        default: return nil
        }
    }
}

enum CoreVoltageRule {
    /// A target at or below this never needs the word, mV.
    static let wordAboveMV: UInt16 = 1200

    /// Millivolts for a pico-sdk `vreg_voltage` code in the range both chips
    /// share (0x06 = 850 mV ... 0x0F = 1300 mV), else nil.
    static func millivolts(_ code: UInt8) -> UInt16? {
        guard (0x06...0x0F).contains(code) else { return nil }
        return 850 + 50 * UInt16(code - 0x06)
    }

    /// The rule.  `currentMV` is nil when the unit's voltage cannot be read.
    static func wordRequired(currentMV: UInt16?, targetMV: UInt16) -> Bool {
        guard targetMV > wordAboveMV else { return false }
        guard let current = currentMV else { return true }
        return targetMV > current
    }

    /// The unit's current voltage: the highest of live, stored and build
    /// default.  nil, meaning unreadable, if any of them is not a voltage.
    static func currentMV(_ s: CoreVregStatus) -> UInt16? {
        let values = [s.liveVreg, s.storedVreg, s.defaultVreg].map(millivolts)
        guard !values.contains(where: { $0 == nil }) else { return nil }
        return values.compactMap { $0 }.max()
    }

    /// What a SET compares with: `currentMV`, but while a trial runs, without
    /// its live voltage, since a SET changes the persisted voltage.
    static func persistedMV(_ s: CoreVregStatus) -> UInt16? {
        guard let current = currentMV(s) else { return nil }
        guard s.trialActive else { return current }
        return [s.storedVreg, s.defaultVreg].compactMap(millivolts).max()
    }

    /// Whether `op` needs the word on the status just read (nil if it could
    /// not be read).  Only SET and TRIAL can raise the voltage.  SET 0 is the
    /// build default, which needs the word only when the unit is unreadable;
    /// so does a code that is not a voltage.
    static func needsWord(_ op: CoreVregOp, status: CoreVregStatus?) -> Bool {
        func judge(_ code: UInt8, _ current: UInt16?) -> Bool {
            guard let target = millivolts(code) else { return true }
            return wordRequired(currentMV: current, targetMV: target)
        }
        switch op {
        case .set(0): return status.flatMap(currentMV) == nil
        case .set(let code): return judge(code, status.flatMap(persistedMV))
        case .trial(let code): return judge(code, status.flatMap(currentMV))
        case .trialEnd, .auto, .clear: return false
        }
    }

    /// True for a tunnelled `EXT_CORE_VREG` write.
    static func isCoreVregWrite(_ request: LinkCmdRequest) -> Bool {
        request.direction == .set && request.bRequest == REQ_EXTENSION && request.wValue == EXT_CORE_VREG
    }

    /// Whether a hub refuses `request` from a network session.  `readStatus`
    /// reads the unit's status now (nil when it cannot); it is called only for
    /// a well-formed SET or TRIAL, the writes that can raise.
    static func refuses(_ request: LinkCmdRequest, readStatus: () -> CoreVregStatus?) -> Bool {
        guard isCoreVregWrite(request) else { return false }
        guard let op = CoreVregOp(payload: request.payload) else { return true }
        switch op {
        case .set, .trial: return needsWord(op, status: readStatus())
        case .trialEnd, .auto, .clear: return false
        }
    }
}
