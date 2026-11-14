//
//  ReadOnlyFloor.swift
//  DSPi Console
//
//  WP P1SW3 (PRO1 S8).  The read-only floor, as Qt's (QD6, QP16): on a
//  device whose firmware speaks another wire format (SW8: shown, never
//  written) or that the installer has locked on this console (P1LK), the main
//  window stays up for reading.  What the state allows stays live; every
//  other control is off and says why in the core's words, instead of taking
//  an edit and showing a refusal after it.
//
//  - Read-only firmware: the core refuses every write, so only reading is
//    left (the meters and the analyser).
//  - The installer's lock: the core's `LOCKED_USER_WRITES` (volume, mute,
//    preset recall) and the analyser.
//

import SwiftUI

/// A control of the main window, by what the floor may leave live.
enum FloorControl: Equatable {
    /// The master or listening volume.
    case volume
    /// An output's mute.
    case mute
    /// Recalling a preset slot (not saving, renaming or clearing one).
    case presetRecall
    /// The analyser and the meters: reading only.
    case analyser
    /// Every other control: a setting.
    case other
}

struct ReadOnlyFloor: Equatable {
    enum Cause: Equatable {
        /// Nothing in the way: every control works.
        case none
        /// Firmware in another wire format (`DspState.compat`).
        case readOnlyFirmware
        /// The installer's lock (the locked-user state).
        case installerLock
    }

    let cause: Cause

    static let open = ReadOnlyFloor(cause: .none)

    /// The floor for a device's state.  Read-only firmware wins over the lock:
    /// it allows less.
    static func of(connected: Bool, deviceReadOnly: Bool, lockedUser: Bool) -> ReadOnlyFloor {
        guard connected else { return .open }
        if deviceReadOnly { return ReadOnlyFloor(cause: .readOnlyFirmware) }
        if lockedUser { return ReadOnlyFloor(cause: .installerLock) }
        return .open
    }

    /// The floor of `vm`'s device under `installer`'s lock.
    static func of(_ vm: DSPViewModel, installer: CoreInstaller) -> ReadOnlyFloor {
        of(connected: vm.isDeviceConnected, deviceReadOnly: vm.deviceReadOnly, lockedUser: installer.lockedUser)
    }

    /// The control works in this state.
    func allows(_ control: FloorControl) -> Bool {
        switch cause {
        case .none: return true
        case .readOnlyFirmware: return control == .analyser
        case .installerLock: return control != .other
        }
    }

    /// Some control is off.
    var isReadOnly: Bool { cause != .none }

    /// The core's row that says why.
    var reasonKey: String {
        switch cause {
        case .none: return ""
        case .readOnlyFirmware: return "read_only"
        case .installerLock: return "installer_lock.banner"
        }
    }

    /// Why a control is off, in the core's words ("" when none is).
    var reason: String {
        guard isReadOnly else { return "" }
        let n = ConsoleNotice.row(reasonKey)
        return n.reason.isEmpty ? n.title : n.reason
    }
}

// MARK: - In the views

private struct ReadOnlyFloorKey: EnvironmentKey {
    static let defaultValue = ReadOnlyFloor.open
}

extension EnvironmentValues {
    /// The window's read-only floor (`ContentView` sets it).
    var readOnlyFloor: ReadOnlyFloor {
        get { self[ReadOnlyFloorKey.self] }
        set { self[ReadOnlyFloorKey.self] = newValue }
    }
}

/// Turns a control off when the floor does not allow it, with the core's
/// reason as its help and accessibility hint.
private struct FloorGate: ViewModifier {
    @Environment(\.readOnlyFloor) private var floor
    let control: FloorControl

    func body(content: Content) -> some View {
        // Only when off, so a live control keeps its own help.  (A change
        // of floor rebuilds the control; it re-reads its value on appear.)
        if !floor.allows(control) {
            content
                .disabled(true)
                .help(floor.reason)
                .accessibilityHint(floor.reason)
        } else {
            content
        }
    }
}

/// The floor's reason as help, only while the control is off: a help
/// modifier on a live control costs the sidebar's rows a layout pass on every
/// update (the Tube Modeller benchmark), so a live control gets none.
private struct FloorHelp: ViewModifier {
    let off: Bool
    let reason: String

    func body(content: Content) -> some View {
        if off {
            content.help(reason).accessibilityHint(reason)
        } else {
            content
        }
    }
}

extension View {
    /// `reason` as the help of a control that is off (`off`); nothing on a
    /// live one.  For controls the floor turns off by other means
    /// (`allowsHitTesting` on a pop-up button).
    func floorHelp(_ off: Bool, _ reason: @autoclosure () -> String) -> some View {
        modifier(FloorHelp(off: off, reason: off ? reason() : ""))
    }

    /// Off (shown, not editable) when the window's read-only floor does not
    /// allow `control`, saying why.
    func floorGated(_ control: FloorControl) -> some View {
        modifier(FloorGate(control: control))
    }
}

/// The line over a read-only page: why its controls are off, in the core's
/// words.  Nothing when every control works.
/// Its padding is its own, so with every control live it takes no room.
struct ReadOnlyFloorLine: View {
    @Environment(\.readOnlyFloor) private var floor
    var verticalPadding: CGFloat = 0

    var body: some View {
        if floor.isReadOnly {
            Label(floor.reason, systemImage: "lock.fill")
                .font(.caption)
                .foregroundColor(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal)
                .padding(.vertical, verticalPadding)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("readOnlyFloorLine")
        }
    }
}
