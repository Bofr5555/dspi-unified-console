//
//  InstallerViews.swift
//  DSPi Console
//
//  WP P1SW2.  The installer's pages and prompts, as Qt's InstallerPage,
//  LockBanner and UnlockDialog and Android's Installer and At Power-On pages
//  (P1QT2, P1AN2): Settings > Installer (the limits profile, the PIN, the
//  lock, Protect all outputs) and Settings > At Power-On (S5), both in the
//  Installer mode only; the locked-user banner over the main window with its
//  one Unlock prompt; the mode switch; the recovery code shown once.  The
//  logic and the words of every refusal are the core's (CoreInstaller).
//

import SwiftUI
import dspi_core

// MARK: - Settings > Installer

struct InstallerSettingsTab: View {
    @ObservedObject private var installer = CoreInstaller.shared
    @ObservedObject private var vm = AppState.shared.viewModel
    @State private var newPin = ""
    @State private var newPinAgain = ""
    @State private var unlockPin = ""
    @State private var output = 0
    @State private var confirmRemovePin = false

    var body: some View {
        Form {
            if !installer.available {
                Section {
                    Text("Connect a DSPi to set its limits and the installer's PIN.")
                        .foregroundColor(.secondary)
                }
            } else if !installer.installerOpen {
                unlockSection
            } else {
                noticeSection
                pinSection
                limitsSection
                outputSection
                protectSection
                textSection
            }
        }
        .formStyle(.grouped)
        .accessibilityIdentifier("installerPage")
        .onAppear { installer.refresh() }
        // The recovery code of the PIN just set: shown once, over this page
        // (the Settings window on the Mac, the Settings sheet on iOS).
        .sheet(isPresented: Binding(get: { installer.recoveryCode != nil },
                                    set: { if !$0 { installer.recoveryCode = nil } })) {
            RecoveryCodeView(installer: installer, code: installer.recoveryCode ?? "")
                .padding(20)
                .frame(minWidth: 320, idealWidth: 420)
                .interactiveDismissDisabled()
        }
        .alert("Remove the installer PIN?", isPresented: $confirmRemovePin) {
            Button("Remove PIN", role: .destructive) { installer.removePin() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Nothing will be locked on this console any more, and the recovery code stops working. The limits stay.")
        }
    }

    // The notice line, the standing notice of a profile that did not read,
    // and what a host-side limit cannot stop (limits.scope).
    @ViewBuilder
    private var noticeSection: some View {
        Section {
            if let notice = installer.notice {
                SnapshotsNoticeLine(notice: notice)
                    .accessibilityIdentifier("installerNotice")
            }
            if let key = installer.status?.noticeKey, !key.isEmpty {
                Text(CoreInstaller.line(key, fallback: installer.status?.detail ?? ""))
                    .foregroundColor(.red)
                    .font(.callout)
            }
            Text("Limits narrow what this DSPi may be set to from this console. A change applies at once; settings already outside a new limit are brought inside it, and what changed is shown.")
                .font(.callout)
            Text(CoreInstaller.reason("limits.scope"))
                .font(.caption)
                .foregroundColor(.secondary)
        }
    }

    private var unlockSection: some View {
        Section {
            Text(installer.scopeLine).font(.callout)
            SecureField("PIN or recovery code", text: $unlockPin)
                .accessibilityIdentifier("installerUnlockPin")
                .onSubmit(unlock)
            HStack {
                Spacer()
                Button(installer.hubHolds ? "Unlock on the hub" : "Unlock", action: unlock)
                    .disabled(unlockPin.isEmpty || installer.busy)
                    .accessibilityIdentifier("installerUnlock")
            }
            if let notice = installer.notice {
                SnapshotsNoticeLine(notice: notice)
            }
        } header: {
            Label("Installer PIN or recovery code", systemImage: "lock")
        }
    }

    private func unlock() {
        let pin = unlockPin
        installer.unlock(pin) { ok in if ok { unlockPin = "" } }
    }

    @ViewBuilder
    private var pinSection: some View {
        Section {
            if installer.hubHolds {
                Text(CoreInstaller.reason("installer_lock.hub")).font(.callout)
                Button("Lock Now") { installer.lockNow() }
                    .accessibilityIdentifier("installerLockNow")
            } else if !installer.lock.pinSet {
                Text("No PIN is set: nothing is locked and nothing asks for one. Set a PIN to lock this console's settings for everyday use; volume, mute and presets stay open.")
                    .font(.callout)
                newPinFields
            } else {
                if installer.lock.recovered {
                    Text("Unlocked with the recovery code. Set a new PIN: remove this one, then set another.")
                        .foregroundColor(.orange)
                }
                Toggle(isOn: Binding(get: { !installer.lock.covers.isEmpty },
                                     set: { installer.setCovered($0) })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Lock the settings")
                        Text(installer.lock.covers.isEmpty
                             ? "With nothing ticked, the PIN guards the Installer pages only."
                             : CoreInstaller.bannerLine)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
                .accessibilityIdentifier("installerLockSettings")
                // Two buttons in one Form row: borderless, so a tap on one is
                // that one only.
                HStack {
                    Button("Lock Now") { installer.lockNow() }
                        .buttonStyle(.borderless)
                        .accessibilityIdentifier("installerLockNow")
                    Spacer()
                    Button("Remove PIN...", role: .destructive) { confirmRemovePin = true }
                        .buttonStyle(.borderless)
                        .accessibilityIdentifier("installerRemovePin")
                }
                Text(installer.scopeLine).font(.caption).foregroundColor(.secondary)
            }
        } header: {
            Label("Installer PIN", systemImage: "key")
        }
        .disabled(installer.busy)
    }

    @ViewBuilder
    private var newPinFields: some View {
        SecureField("New PIN (4 to 64 characters)", text: $newPin)
            .accessibilityIdentifier("installerNewPin")
        SecureField("The same again", text: $newPinAgain)
            .accessibilityIdentifier("installerNewPinAgain")
        HStack {
            Spacer()
            Button("Set PIN") {
                installer.setPin(newPin, again: newPinAgain)
                newPin = ""
                newPinAgain = ""
            }
            .disabled(newPin.isEmpty || newPinAgain.isEmpty)
            .accessibilityIdentifier("installerSetPin")
        }
    }

    private var limitsSection: some View {
        Section {
            LimitFieldRow(title: "Master volume at most", key: "masterVolumeMaxDb",
                          value: installer.draft.value("masterVolumeMaxDb")) { installer.setLimit("masterVolumeMaxDb", $0) }
            LimitFieldRow(title: "EQ boost at most, every band", key: "maxPeqBoostDb",
                          value: installer.draft.value("maxPeqBoostDb")) { installer.setLimit("maxPeqBoostDb", $0) }
            HStack {
                Spacer()
                Button("Remove All Limits") { installer.clearLimits() }
                    .disabled(installer.draft.isEmpty)
                    .accessibilityIdentifier("installerClearLimits")
            }
        } header: {
            Label("Limits", systemImage: "slider.horizontal.below.rectangle")
        }
    }

    private var outputSection: some View {
        Section {
            Picker("Output", selection: $output) {
                ForEach(0..<max(1, installer.outputCount), id: \.self) { o in
                    Text(installer.outputName(o)).tag(o)
                }
            }
            .accessibilityIdentifier("installerOutput")
            ForEach(Self.outputFields) { field in
                LimitFieldRow(title: field.title, key: field.key,
                              value: installer.draft.outputValue(output, field.key)) {
                    installer.setOutputLimit(output, field.key, $0)
                }
            }
        } header: {
            Label("Each output", systemImage: "cable.connector")
        }
    }

    /// Each output's limits, in the profile's order (the words; the ranges
    /// are the core's).
    static let outputFields: [LimitField] = [
        LimitField(key: "maxGainDb", title: "Gain at most"),
        LimitField(key: "limiterCeilingDb", title: "Limiter on, ceiling at most"),
        LimitField(key: "highpassMinHz", title: "High-pass kept, at or above"),
        LimitField(key: "highpassMinSlopeDb", title: "High-pass slope at least"),
        LimitField(key: "delayMinMs", title: "Delay at least"),
        LimitField(key: "delayMaxMs", title: "Delay at most"),
    ]

    @ViewBuilder
    private var protectSection: some View {
        Section {
            if !installer.withoutLimiter.isEmpty {
                Text("\(installer.outputNames(installer.withoutLimiter)): above -10 dB with no limiter.")
                    .foregroundColor(.orange)
                    .accessibilityIdentifier("installerWithoutLimiter")
            }
            HStack {
                Text("Every output's limiter on at -1 dBFS, stereo pairs linked. One undo step.")
                    .font(.callout)
                Spacer()
                Button("Protect all outputs") { installer.protectAllOutputs() }
                    .accessibilityIdentifier("installerProtectAll")
            }
        } header: {
            Label("Limiters", systemImage: "shield")
        }
        .disabled(installer.busy)
    }

    private var textSection: some View {
        Section {
            LimitTextRow(title: "Note", placeholder: "What this system is set up for",
                         text: installer.draft.text("note")) { installer.setText("note", $0) }
            LimitTextRow(title: "Contact", placeholder: "How to reach the installer",
                         text: installer.draft.text("contact")) { installer.setText("contact", $0) }
        } header: {
            Label("Installer's note", systemImage: "note.text")
        }
    }
}

/// One limit: a number in the core's range for `key`, empty for none.
/// Applied when the field is submitted or left.
struct LimitFieldRow: View {
    let title: String
    let key: String
    let value: Float?
    let onSet: (Float?) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    private var range: LimitFieldRange? { CoreInstaller.shared.range(key) }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let r = range {
                    Text("\(Self.format(r.min)) to \(Self.format(r.max)) \(r.unit)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
            Spacer()
            TextField("No limit", text: $text)
                .multilineTextAlignment(.trailing)
                .frame(maxWidth: 110)
                .focused($focused)
                .onSubmit(commit)
                #if os(iOS)
                .keyboardType(.numbersAndPunctuation)
                #endif
                .accessibilityIdentifier("limit.\(key)")
        }
        .onAppear { text = Self.text(value) }
        .onChange(of: value) { v in if !focused { text = Self.text(v) } }
        .onChange(of: focused) { f in if !f { commit() } }
    }

    private func commit() {
        let t = text.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: ".")
        let next: Float? = t.isEmpty ? nil : Float(t)
        if !t.isEmpty, next == nil {   // not a number: back to what is set
            text = Self.text(value)
            return
        }
        let held = next.map { CoreInstaller.shared.clamp(key, $0) }
        text = Self.text(held)
        if held != value { onSet(held) }
    }

    static func text(_ v: Float?) -> String { v.map(format) ?? "" }

    static func format(_ v: Float) -> String {
        v == v.rounded() ? String(Int(v)) : String(format: "%.1f", v)
    }
}

/// The installer's note or contact.
struct LimitTextRow: View {
    let title: String
    let placeholder: String
    let text: String
    let onSet: (String) -> Void
    @State private var edit = ""
    @FocusState private var focused: Bool

    var body: some View {
        TextField(title, text: $edit, prompt: Text(placeholder))
            .focused($focused)
            .onSubmit { onSet(edit) }
            .onAppear { edit = text }
            .onChange(of: text) { t in if !focused { edit = t } }
            .onChange(of: focused) { f in if !f, edit != text { onSet(edit) } }
            .accessibilityIdentifier("limitText.\(title)")
    }
}

// MARK: - Settings > At Power-On (S5)

struct PowerOnSettingsTab: View {
    @ObservedObject private var vm = AppState.shared.viewModel
    @State private var confirmTest = false
    @State private var result: String?

    /// -1: the last used preset; 0...: that slot.
    private var startup: Binding<Int> {
        Binding(get: { vm.presetStartupMode == 0 ? vm.presetDefaultSlot : -1 },
                set: { v in
                    if v < 0 { vm.setPresetStartup(mode: 1, defaultSlot: vm.presetDefaultSlot) }
                    else { vm.setPresetStartup(mode: 0, defaultSlot: v) }
                })
    }

    /// The slot a power-on loads: the default, or the one in use.
    private var startupSlot: Int { vm.presetStartupMode == 0 ? vm.presetDefaultSlot : vm.activePresetSlot }

    var body: some View {
        Form {
            if !vm.isDeviceConnected {
                Section { Text("No device.").foregroundColor(.secondary) }
            } else {
                Section {
                    Text("What the DSPi does when it starts. Changes apply at once.")
                        .font(.callout)
                    Picker("Startup Preset", selection: startup) {
                        Text("Last Used").tag(-1)
                        ForEach(0..<vm.presetNames.count, id: \.self) { slot in
                            Text(slotLabel(slot)).tag(slot)
                        }
                    }
                    .accessibilityIdentifier("powerOnStartup")
                    Picker("Master Volume", selection: Binding(get: { vm.presetMasterVolumeMode },
                                                               set: { vm.setMasterVolumeMode($0) })) {
                        Text("Kept apart from presets, applied at power-on").tag(MASTER_VOLUME_MODE_INDEPENDENT)
                        Text("Part of each preset").tag(MASTER_VOLUME_MODE_WITH_PRESET)
                    }
                    .accessibilityIdentifier("powerOnMasterVolume")
                    Picker("Hardware Configuration", selection: Binding(get: { vm.presetOutputConfigMode },
                                                                        set: { vm.setOutputConfigMode($0) })) {
                        Text("Kept apart from presets, applied at power-on").tag(OUTPUT_CONFIG_MODE_INDEPENDENT)
                        Text("Part of each preset").tag(OUTPUT_CONFIG_MODE_WITH_PRESET)
                    }
                    .accessibilityIdentifier("powerOnHardware")
                    Text("S/PDIF and ADAT inputs stay muted until they lock to a signal, so a start with no source is silent.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Text("Auxiliary outputs: their states are saved with the control surfaces and come back at power-on.")
                        .font(.caption)
                        .foregroundColor(.secondary)
                } header: {
                    Label("At Power-On", systemImage: "power")
                }
                Section {
                    HStack {
                        Text("Loads preset \(slotLabel(startupSlot)), the one the DSPi starts with.")
                            .font(.callout)
                        Spacer()
                        Button("Test Power-On...") { confirmTest = true }
                            .accessibilityIdentifier("powerOnTest")
                    }
                    if let result = result {
                        Text(result).font(.callout).accessibilityIdentifier("powerOnResult")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .accessibilityIdentifier("powerOnPage")
        .alert("Test Power-On?", isPresented: $confirmTest) {
            Button("Load") { testPowerOn() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This loads preset \(slotLabel(startupSlot)) as the DSPi does when it starts. Changes not saved to a preset are lost.")
        }
    }

    private func slotLabel(_ slot: Int) -> String {
        let name = slot < vm.presetNames.count ? vm.presetNames[slot] : ""
        return name.isEmpty ? "\(slot + 1)" : "\(slot + 1) (\(name))"
    }

    private func testPowerOn() {
        let slot = startupSlot
        DispatchQueue.global(qos: .userInitiated).async {
            let status = vm.loadPreset(slot: slot)
            DispatchQueue.main.async {
                result = status == PRESET_OK ? "Preset \(slot + 1) loaded, as at power-on."
                                             : "The preset did not load."
            }
        }
    }
}

// MARK: - The locked user's Settings page

struct InstallerNoteTab: View {
    @ObservedObject private var installer = CoreInstaller.shared
    @State private var pin = ""

    var body: some View {
        Form {
            Section {
                Text(CoreInstaller.bannerLine)
                if !installer.lock.note.isEmpty { Text(installer.lock.note) }
                if !installer.lock.contact.isEmpty { Text("Contact: \(installer.lock.contact)") }
            } header: {
                Label("Installer's note", systemImage: "note.text")
            }
            Section {
                Text(installer.scopeLine).font(.caption).foregroundColor(.secondary)
                SecureField("PIN or recovery code", text: $pin)
                    .accessibilityIdentifier("installerNoteUnlockPin")
                HStack {
                    Spacer()
                    Button(installer.hubHolds ? "Unlock on the hub" : "Unlock") {
                        installer.unlock(pin) { _ in pin = "" }
                    }
                    .disabled(pin.isEmpty || installer.busy)
                }
                if let notice = installer.notice, notice.kind != .info { SnapshotsNoticeLine(notice: notice) }
            } header: {
                Label("Installer PIN or recovery code", systemImage: "lock")
            }
        }
        .formStyle(.grouped)
        .accessibilityIdentifier("installerNotePage")
    }
}

// MARK: - The main window: banner and prompts

/// The locked-user state over the main window: the core's line, the
/// installer's note and contact, Unlock.  Unlocked with a PIN set in the
/// Installer mode: a slim "Installer" line with Lock Now.
struct LockBanner: View {
    @ObservedObject var installer: CoreInstaller

    var body: some View {
        if installer.lockedUser {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "lock.fill").accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(CoreInstaller.bannerLine).font(.callout)
                    if !installer.lock.note.isEmpty {
                        Text(installer.lock.note).font(.caption)
                    }
                    if !installer.lock.contact.isEmpty {
                        Text("Installer: \(installer.lock.contact)").font(.caption).foregroundColor(.secondary)
                    }
                }
                Spacer(minLength: 0)
                Button("Unlock...") { installer.askUnlock() }
                    .accessibilityIdentifier("lockBannerUnlock")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color.orange.opacity(0.15))
            .overlay(Divider(), alignment: .bottom)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("lockBanner")
        } else if installer.unlockedInstaller {
            HStack(spacing: 8) {
                Text("Installer").font(.callout.bold())
                Text("Unlocked on this console for this session.").font(.caption).foregroundColor(.secondary)
                Spacer(minLength: 0)
                Button("Lock Now") { installer.lockNow() }
                    .accessibilityIdentifier("installerBadgeLockNow")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 4)
            .background(Color.accentColor.opacity(0.10))
            .overlay(Divider(), alignment: .bottom)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("installerBadge")
        }
    }
}

/// The one unlock prompt: the PIN or the recovery code, on this console or
/// on the hub that holds the lock; the lock's scope in the core's words.
struct UnlockSheet: View {
    @ObservedObject var installer: CoreInstaller
    @State private var pin = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(installer.hubHolds ? "Unlock on the hub" : "Unlock this console").font(.headline)
            Text(installer.scopeLine).font(.callout).fixedSize(horizontal: false, vertical: true)
            SecureField("PIN or recovery code", text: $pin)
                .onSubmit(unlock)
                .accessibilityIdentifier("unlockPin")
            if let notice = installer.notice, notice.kind != .info {
                SnapshotsNoticeLine(notice: notice).accessibilityIdentifier("unlockNotice")
            }
            HStack {
                Spacer()
                Button("Cancel") { installer.unlockShown = false }
                    .keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("unlockCancel")
                Button("Unlock", action: unlock)
                    .keyboardShortcut(.defaultAction)
                    .disabled(pin.isEmpty || installer.busy)
                    .accessibilityIdentifier("unlockButton")
            }
        }
        .padding(20)
        .frame(minWidth: 320, idealWidth: 420)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("unlockSheet")
    }

    private func unlock() {
        guard !pin.isEmpty else { return }
        installer.unlock(pin) { ok in
            pin = ""
            if ok { installer.unlockShown = false }
        }
    }
}

/// The mode switch: the manifest's modes, for this device or every device
/// on this console.  The Installer asks for the PIN here when one is set
/// and this console is not unlocked; with no PIN nothing asks.  As a sheet
/// (the Mac's Mode... menu item) or a Settings page (Settings > Mode).
struct ModeSwitchView: View {
    @ObservedObject var installer: CoreInstaller
    /// Shown as a sheet: Cancel, and Switch closes it.
    var isSheet = true
    @State private var picked = ""
    @State private var everyDevice = false
    @State private var pin = ""

    private var needsPin: Bool {
        let entry = installer.modes.first { $0.id == picked }
        return (entry?.needsPinWhenSet ?? false) && installer.lock.pinSet && !installer.installerOpen
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if isSheet { Text("Mode").font(.headline) }
            Text("Now: \(installer.label(ofMode: installer.mode))")
                .font(.callout)
                .accessibilityIdentifier("modeNow")
            if !installer.switchable {
                Text(reasonLine).font(.callout).accessibilityIdentifier("modeFixedReason")
            }
            Picker("Mode", selection: $picked) {
                ForEach(installer.modes, id: \.id) { m in Text(m.label).tag(m.id) }
            }
            #if os(macOS)
            .pickerStyle(.radioGroup)
            #else
            .pickerStyle(.segmented)
            #endif
            .labelsHidden()
            .disabled(!installer.switchable)
            .accessibilityIdentifier("modePicker")
            Toggle("Use for every device on this console", isOn: $everyDevice)
                .disabled(!installer.switchable)
                .accessibilityIdentifier("modeEveryDevice")
            if needsPin {
                Text(installer.scopeLine).font(.caption).foregroundColor(.secondary)
                SecureField("PIN or recovery code", text: $pin)
                    .accessibilityIdentifier("modePin")
                if let notice = installer.notice, notice.kind != .info {
                    SnapshotsNoticeLine(notice: notice)
                }
            }
            HStack {
                Spacer()
                if isSheet {
                    Button("Cancel") { installer.modeShown = false }
                        .keyboardShortcut(.cancelAction)
                }
                Button("Switch", action: switchMode)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!installer.switchable || picked.isEmpty || picked == installer.mode
                              || (needsPin && pin.isEmpty) || installer.busy)
                    .accessibilityIdentifier("modeSwitchApply")
            }
        }
        .padding(isSheet ? 20 : 0)
        .frame(minWidth: 280, idealWidth: 380, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modeSwitch")
        .onAppear { picked = installer.mode == "locked_user" ? CoreInstaller.defaultMode(installer.manifest) : installer.mode }
    }

    private func switchMode() {
        let mode = picked, every = everyDevice
        let done = {
            installer.choose(mode, everyDevice: every)
            if isSheet { installer.modeShown = false }
        }
        guard needsPin else { return done() }
        installer.unlock(pin) { ok in
            pin = ""
            if ok { done() }
        }
    }

    private var reasonLine: String {
        switch installer.choice?.reason {
        case "pinned": return "The hub sets this console's mode for this DSPi: \(installer.label(ofMode: installer.mode))."
        case "locked": return CoreInstaller.bannerLine
        default: return ""
        }
    }
}

/// Settings > Mode: the switch as a page.
struct ModeSettingsTab: View {
    @ObservedObject private var installer = CoreInstaller.shared

    var body: some View {
        Form {
            Section {
                ModeSwitchView(installer: installer, isSheet: false)
            } header: {
                Label("Mode", systemImage: "slider.horizontal.3")
            }
            Section {
                Text("Simple shows the everyday controls; Advanced shows everything; Installer adds the limits, the installer's PIN and the power-on settings.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .formStyle(.grouped)
        .accessibilityIdentifier("modePage")
    }
}

/// The recovery code, shown once when the PIN is set (at the top of the
/// Installer page, until the installer says it is written down).
struct RecoveryCodeView: View {
    @ObservedObject var installer: CoreInstaller
    let code: String

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Recovery code").font(.headline)
            Text(code)
                .font(.system(.title3, design: .monospaced))
                .textSelection(.enabled)
                .accessibilityIdentifier("installerRecoveryCode")
            Text(CoreInstaller.line("installer_lock.recovery")).font(.callout).fixedSize(horizontal: false, vertical: true)
            let action = noticeForKey(key: "installer_lock.recovery", detail: "").action
            if !action.isEmpty { Text(action).font(.caption).foregroundColor(.secondary) }
            HStack {
                Spacer()
                Button("I Have Written It Down") { installer.recoveryCode = nil }
                    .buttonStyle(.borderless)
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("installerRecoveryDone")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("installerRecovery")
    }
}

/// The installer's sheets over the main window: the unlock prompt and the
/// mode switch.
struct InstallerPrompts: ViewModifier {
    @ObservedObject var installer: CoreInstaller

    func body(content: Content) -> some View {
        content
            .sheet(isPresented: $installer.unlockShown) { UnlockSheet(installer: installer) }
            .sheet(isPresented: $installer.modeShown) { ModeSwitchView(installer: installer) }
    }
}

extension View {
    /// -- WP P1SW2 -- The installer's prompts over this view.
    func installerPrompts(_ installer: CoreInstaller = .shared) -> some View {
        modifier(InstallerPrompts(installer: installer))
    }
}

/// One field of the profile and its words.
struct LimitField: Identifiable, Hashable {
    let key: String
    let title: String
    var id: String { key }
}
