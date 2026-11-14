import SwiftUI

// MARK: - Sidebar Selection

enum SidebarSelection: Hashable {
    case overview
    case input(Int)   // Input EQ channel index 0..chOut1-1
    case output(Int)  // Matrix output index 0-8
}

/// Filter types offered to the user in the PEQ picker, gated on connected-firmware
/// capability.  Crossover filter types live in their own picker on the Crossover
/// tab and never appear here.  Notch / AllPass were added in firmware 1.1.4 -
/// older firmware would reject the type byte, so they're hidden until we've
/// confirmed support.  The first-order all-pass (wire V13) and first-order
/// shelves (wire V14) and the first-order low/high pass (V28) are gated on the
/// bulk wire-format version, since the firmware release version did not
/// advance with them.
///
/// `includeLinkwitz` is false for input channels: the Linkwitz Transform is a
/// driver/sealed-box bass-extension tool that only makes sense on the outputs
/// feeding physical speakers, so it's hidden from input EQ banks entirely.
func availableFilterTypes(vm: DSPViewModel, includeLinkwitz: Bool = true) -> [FilterType] {
    var filters: [FilterType] = FilterType.allCases.filter { !$0.isCrossover }

    if vm.firmwareSupportsNotch == false {
        filters = filters.filter { $0 != .notch }
    }

    if vm.firmwareSupportsAllPass == false {
        filters = filters.filter { $0 != .allPass }
    }

    if vm.firmwareSupportsFirstOrderAllPass == false {
        filters = filters.filter { $0 != .allPass1 }
    }

    if vm.firmwareSupportsFirstOrderShelves == false {
        filters = filters.filter { $0 != .lowShelf1 && $0 != .highShelf1 }
    }

    if vm.firmwareSupportsFirstOrderPass == false {
        filters = filters.filter { $0 != .lowPass1 && $0 != .highPass1 }
    }

    if vm.firmwareSupportsLinkwitzTransform == false || includeLinkwitz == false {
        filters = filters.filter { $0 != .linkwitzTransform }
    }
    return filters
}

// MARK: - Main Layout

struct ContentView: View {
    #if os(iOS)
    fileprivate static var didApplyLaunchSelection = false
    #endif
    @ObservedObject var vm: DSPViewModel
    @ObservedObject private var settings = AppSettings.shared
    // -- WP P1SW3 -- the installer's lock, for the read-only floor
    @ObservedObject private var installer = CoreInstaller.shared
    @EnvironmentObject var matrixMixerController: MatrixMixerWindowController
    @EnvironmentObject var loudnessController: LoudnessWindowController
    @EnvironmentObject var crossfeedController: CrossfeedWindowController
    @EnvironmentObject var statsController: StatsWindowController
    @EnvironmentObject var graphWindowController: GraphWindowController
    @EnvironmentObject var levellerController: VolumeLevellerWindowController
    @EnvironmentObject var psybassController: PsychoacousticBassWindowController
    #if os(macOS)   // -- WP SW18 -- no firmware window on iOS
    @EnvironmentObject var firmwareUpdateController: FirmwareUpdateWindowController
    #endif
    /// Read only so the basics tour can put the right pane on screen for the
    /// steps that describe it; the console is otherwise unaware of onboarding.
    @EnvironmentObject var onboarding: OnboardingCoordinator
    @State private var selection: SidebarSelection = .overview
    @State private var renamingChannel: Int? = nil  // channelNames index

    /// At higher input-channel counts the sidebar gets crowded, so tighten
    /// each row's height to keep all inputs plus outputs comfortably visible.
    /// Graduated: a little tighter at 6-ch, tighter still at 8-ch.
    /// -- BUG1 -- The sidebar's name column: 80 as before, wide enough for a derived output
    /// label ("Out 2 · R · High") while one shows, so every row's meter starts at one place.
    private var sidebarNameWidth: CGFloat {
        let derived = vm.outputLabelTexts.indices.contains { o in
            let ch = vm.eqChannel(forOutput: o)
            let stored = ch < vm.channelNames.count ? vm.channelNames[ch] : ""
            return o < vm.outputEnabled.count && vm.outputEnabled[o] && vm.outputLabelTexts[o] != stored
        }
        guard derived else { return 80 }
        #if os(iOS)
        // -- review c86 F2/F3 -- iOS rows show the short form ("R · High"): the column fits
        // the widest one shown at the body size in use (Dynamic Type), from 80 to 140 pt.
        let font = UIFont.preferredFont(forTextStyle: .body)
        let widest = vm.outputEnabled.indices.filter { vm.outputEnabled[$0] }
            .map { (vm.outputRowLabel($0) as NSString).size(withAttributes: [.font: font]).width }
            .max() ?? 0
        return min(max(80, ceil(widest) + 2), 140)
        #else
        return 112
        #endif
    }

    /// -- BUG1 -- VoiceOver's name for an output's pill: "OUT1, Out 1, R, Low, slot I2S 1 L".
    private func outputPillName(_ out: MatrixOutput) -> String {
        let slot = vm.outputSlotName(out.index)
        return [out.descriptor, vm.outputLabel(out.index).replacingOccurrences(of: " · ", with: ", "),
                slot.isEmpty ? "" : "slot \(slot)"].filter { !$0.isEmpty }.joined(separator: ", ")
    }

    /// -- WP P1SW3 -- What the window may change now: everything, or on
    /// read-only firmware or under the installer's lock only what that allows.
    private var floor: ReadOnlyFloor { ReadOnlyFloor.of(vm, installer: installer) }

    private var sidebarRowHeight: CGFloat {
        switch vm.numMatrixInputs {
        case 8...: return 23
        case 6...: return 27
        default:   return 28
        }
    }
    @State private var renameText = ""
    @State private var showPresetRename = false
    @State private var presetRenameSlot = 0
    @State private var presetRenameText = ""
    @State private var showSourceRename = false
    @State private var sourceRenameTarget = 0
    @State private var sourceRenameText = ""
    @State private var presetSwitchInFlight = false
    @State private var settingsWindowOpen = false

    /// Whether the tour opened the Matrix Mixer, so it is only closed again if
    /// the tour is what put it on screen.  A user who already had it open keeps
    /// it when the tour moves on.
    @State private var tourOpenedMatrixMixer = false
    @Environment(\.openWindow) private var openWindow

    private func commitRename() {
        guard let idx = renamingChannel else { return }
        let trimmed = renameText.trimmingCharacters(in: .whitespaces)
        if !trimmed.isEmpty {
            // Bind to the device the rename was typed against: a switch
            // landing before the block runs must not rename the new device's
            // channel.
            let generation = vm.transport.generation
            DispatchQueue.global(qos: .userInitiated).async {
                guard vm.transport.generation == generation else { return }
                vm.setChannelName(channel: idx, name: trimmed)
            }
        }
        renamingChannel = nil
    }

    /// Puts the window the current tour step lives in on screen, and takes it
    /// away again when the tour moves on or ends.
    ///
    /// The Matrix Mixer is floated while it hosts a step: the console is dimmed
    /// and inert underneath, but a click on a dimmed window still raises it in
    /// AppKit, which would bury the grid the step is pointing at.
    private func syncTourWindows() {
        let wantsMatrix = onboarding.basicsTourRunning
            && onboarding.basicsTourStep?.host == .matrixMixer

        if wantsMatrix {
            if !matrixMixerController.isVisible {
                tourOpenedMatrixMixer = true
                matrixMixerController.show()
            }
            matrixMixerController.setFloating(true)
        } else {
            matrixMixerController.setFloating(false)
            if tourOpenedMatrixMixer {
                tourOpenedMatrixMixer = false
                matrixMixerController.hide()
            }
        }
    }

    /// Right-click menu for the sidebar source picker; nil on firmware without
    /// source names, which leaves the picker's default right-click alone.
    private func sourceContextMenu() -> NSMenu? {
        let source = vm.inputSource
        guard vm.inputSourceNamesSupported, vm.inputSourceNames.indices.contains(source),
              floor.allows(.other) else { return nil }   // -- WP SWF4 --, P1SW3
        let menu = NSMenu()
        menu.addItem(ClosureMenuItem(title: "Rename \u{201C}\(vm.inputSourceTitle(source))\u{201D}\u{2026}") {
            sourceRenameTarget = source
            sourceRenameText = vm.inputSourceNames[source]
            showSourceRename = true
        })
        if !vm.inputSourceNames[source].isEmpty {
            menu.addItem(ClosureMenuItem(title: "Use Built-in Name") { commitSourceRename(source, "") })
        }
        return menu
    }

    private func commitSourceRename(_ source: Int, _ name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        // An explicit rename supersedes any unsaved edit in Settings.
        SettingsSaveCoordinator.shared.sourceNameEdits[source] = nil
        guard trimmed != vm.inputSourceNames[source] else { return }
        // Bound to the device the menu was opened on, like channel renames.
        let generation = vm.transport.generation
        DispatchQueue.global(qos: .userInitiated).async {
            guard vm.transport.generation == generation else { return }
            vm.setInputSourceName(source: source, name: trimmed)
        }
    }

    private func startRename(_ channelIdx: Int) {
        guard floor.allows(.other) else { return }   // -- WP SWF4 --, P1SW3
        if renamingChannel != nil { commitRename() }
        renameText = vm.channelNames[channelIdx]
        renamingChannel = channelIdx
    }

    /// Show/hide one channel's curve on the frequency response graph.  The
    /// sidebar descriptor pills are the only control for this; selection still
    /// drives visibility on its own (solo on select, all curves in overview).
    private func toggleCurve(_ eqCh: Int) {
        vm.channelVisibility[eqCh] = !(vm.channelVisibility[eqCh] ?? true)
    }

    private func presetLabel(_ slot: Int) -> String {
        let display = slot + 1
        return "\(display): \(presetDropdownLabel(slot))"
    }

    private func presetDropdownLabel(_ slot: Int) -> String {
        guard vm.isPresetOccupied(slot) else { return "Empty" }
        let trimmed = vm.presetNames[slot].trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Preset \(slot + 1)" : trimmed
    }

    private func saveActivePreset(vm: DSPViewModel) {
        let slot = vm.activePresetSlot
        DispatchQueue.global(qos: .userInitiated).async {
            if vm.presetNames[slot].isEmpty {
                vm.setPresetName(slot: slot, name: "Preset \(slot + 1)")
            }
            let status = vm.savePreset(slot: slot, confirmed: .savePreset())
            if status != PRESET_OK {
                DispatchQueue.main.async {
                    NoticeCenter.main.post(AppNotices.preset(status, "Failed to save preset"))   // -- WP P1SW3 -- was a modal
                }
            }
        }
    }

    /// Copy the current preset's parameters into another slot.
    ///
    /// Implementation:
    ///  1. If there are unsaved changes, prompt with the standard alert
    ///     (Save / Discard / Cancel) so the user picks how to handle them.
    ///  2. Save the live state to `destinationSlot` (writes the current
    ///     parameters into that slot's flash sector).
    ///  3. Re-save the live state back to the source slot.  This is the
    ///     trick that keeps the active selection (and the firmware-side
    ///     `last_active_slot`) on the source — `preset_save` always sets
    ///     `last_active = slot`, so without this second write the firmware
    ///     would consider the destination slot active and a boot in
    ///     `LAST_ACTIVE` startup mode would land on the wrong preset.
    ///     Data-wise it's a no-op (writes the same bytes back to source).
    private func copyActivePreset(to destinationSlot: Int) {
        let sourceSlot = vm.activePresetSlot
        guard destinationSlot != sourceSlot else { return }

        vm.settleUnsavedChanges()   // -- WP SWF7 --
        if vm.isDeviceConnected && vm.hasUnsavedChanges {
            #if os(macOS)
            let action = PresetAlerts.showUnsavedChangesAlert(diff: vm.computeDiff())
            switch action {
            case .save:    copyActivePreset(from: sourceSlot, to: destinationSlot, preOp: .save)
            case .discard: copyActivePreset(from: sourceSlot, to: destinationSlot, preOp: .discard)
            case .cancel:  return
            }
            #else
            PresetAlerts.showUnsavedChangesAlert(diff: vm.computeDiff()) { action in
                switch action {
                case .save:    copyActivePreset(from: sourceSlot, to: destinationSlot, preOp: .save)
                case .discard: copyActivePreset(from: sourceSlot, to: destinationSlot, preOp: .discard)
                case .cancel:  return
                }
            }
            #endif
        } else {
            copyActivePreset(from: sourceSlot, to: destinationSlot, preOp: .none)
        }
    }

    private enum PresetCopyPreOp { case save, discard, none }

    /// The copy itself, once any unsaved changes have been dealt with.
    private func copyActivePreset(from sourceSlot: Int, to destinationSlot: Int, preOp: PresetCopyPreOp) {

        // Scope the whole flow to the device it started on - the deferred
        // flash saves give a device switch time to land mid-flow.
        let generation = vm.transport.generation
        DispatchQueue.global(qos: .userInitiated).async {
            // Pre-op: align live state to source-slot semantics if needed.
            switch preOp {
            case .save:
                if vm.presetNames[sourceSlot].isEmpty {
                    vm.setPresetName(slot: sourceSlot, name: "Preset \(sourceSlot + 1)")
                }
                _ = vm.savePreset(slot: sourceSlot, confirmed: .savePreset())
            case .discard:
                _ = vm.loadPreset(slot: sourceSlot)
            case .none:
                break
            }

            guard vm.transport.generation == generation else { return }

            // Default the destination's name if it has none yet, so the
            // dropdown stops showing "Empty" for it post-copy.  We don't
            // overwrite an existing destination name — the destination
            // keeps its identity, only its parameters change.
            if vm.presetNames[destinationSlot].isEmpty {
                vm.setPresetName(slot: destinationSlot, name: "Preset \(destinationSlot + 1)")
            }

            // copyPreset handles the deferred-save sequencing: save→wait→
            // save→wait, so the firmware actually processes both slot
            // writes instead of the second one overwriting the first's
            // pending slot before main-loop dispatch.
            let status = vm.copyPreset(from: sourceSlot, to: destinationSlot, confirmed: .copyPreset())
            guard vm.transport.generation == generation else { return }
            guard status == PRESET_OK else {
                DispatchQueue.main.async {
                    NoticeCenter.main.post(AppNotices.preset(status, "Failed to copy preset"))   // -- WP P1SW3 -- was a modal
                }
                return
            }
        }
    }

    private func showPresetClearConfirmation(vm: DSPViewModel, slot: Int) {
        #if os(macOS)
        let alert = NSAlert()
        #else
        let alert = PlatformAlert()
        #endif
        alert.messageText = "Clear Preset?"
        alert.informativeText = "Clear \"\(presetLabel(slot))\" and restore factory defaults? This cannot be undone."
        alert.alertStyle = .critical
        alert.addButton(withTitle: "Clear")
        alert.addButton(withTitle: "Cancel")

        #if os(macOS)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        clearPreset(vm: vm, slot: slot)
        #else
        alert.present { if $0 == 0 { clearPreset(vm: vm, slot: slot) } }
        #endif
    }

    private func clearPreset(vm: DSPViewModel, slot: Int) {
        DispatchQueue.global(qos: .userInitiated).async {
            // Scope the whole flow to the device it started on: the flash
            // wait is long enough for a device switch to land mid-flow, and
            // the follow-up reload must never run against the new device.
            let generation = vm.transport.generation
            let status = vm.deletePreset(slot: slot, confirmed: .deletePreset())
            if status == PRESET_OK {
                // Wait for the firmware's deferred delete to complete before
                // any followup reads — otherwise fetchPresetDirectory would
                // observe pre-delete state (slot still occupied) and overwrite
                // our optimistic local "empty" with that, leaving the UI
                // showing the slot as still populated.  Firmware clears the
                // slot name during preset_delete itself, so we don't need a
                // separate setPresetName(slot, "") call.
                _ = vm.waitForPresetDeletion(slot: slot)
                if vm.transport.generation == generation {
                    vm.fetchPresetDirectory()
                    // If we cleared the active slot, reload it to apply factory defaults
                    if vm.activePresetSlot == slot {
                        _ = vm.loadPreset(slot: slot)
                    }
                }
            }
            DispatchQueue.main.async {
                if status != PRESET_OK {
                    NoticeCenter.main.post(AppNotices.preset(status, "Failed to clear preset"))   // -- WP P1SW3 -- was a modal
                }
            }
        }
    }

    private func showPresetClearAllConfirmation(vm: DSPViewModel) {
        #if os(macOS)
        let alert = NSAlert()
        #else
        let alert = PlatformAlert()
        #endif
        alert.messageText = "Clear All Presets?"
        alert.informativeText = "This will erase all preset data and names, restoring every slot to factory defaults. This cannot be undone."
        alert.alertStyle = .critical
        alert.addButton(withTitle: "Clear All")
        alert.addButton(withTitle: "Cancel")

        #if os(macOS)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        clearAllPresets(vm: vm)
        #else
        alert.present { if $0 == 0 { clearAllPresets(vm: vm) } }
        #endif
    }

    private func clearAllPresets(vm: DSPViewModel) {
        DispatchQueue.global(qos: .userInitiated).async {
            // Scope the whole flow to the device it started on (see the
            // single-slot clear above) - with up to 10 slots to drain, this
            // is the longest window for a mid-flow device switch.
            let generation = vm.transport.generation
            // Enqueue a delete for every occupied slot — firmware accumulates
            // them all into preset_delete_mask and processes them in a single
            // main-loop pass.  Wait once for everything to drain by polling
            // each slot's bit until cleared (or timeout).  Same race rationale
            // as the single-slot clear path.
            let toDelete = (0..<10).filter { vm.isPresetOccupied($0) }
            for slot in toDelete {
                vm.deletePreset(slot: slot, confirmed: .deletePreset())
            }
            for slot in toDelete {
                _ = vm.waitForPresetDeletion(slot: slot)
            }
            guard vm.transport.generation == generation else { return }
            vm.fetchPresetDirectory()
            // Reload active slot to apply factory defaults.
            _ = vm.loadPreset(slot: vm.activePresetSlot)
        }
    }

    var body: some View {
        HSplitView {
            // SIDEBAR
            List {
                Section(header: Text("INPUTS")) {
                    // Show exactly the live active input count (2/4/6/8).  Each
                    // input is a first-class EQ channel (index == channel index).
                    ForEach(0..<vm.numMatrixInputs, id: \.self) { ch in
                        // Both rows of a linked pair highlight together if
                        // either is selected.  The right pane still tracks the
                        // actually-clicked channel.
                        let rowSelected: Bool = {
                            if selection == .input(ch) { return true }
                            if let partner = vm.linkedPartner(of: ch) {
                                return selection == .input(partner)
                            }
                            return false
                        }()
                        ChannelRow(channelIndex: ch,
                                  color: MatrixInput.color(for: ch),
                                  descriptor: "IN\(ch + 1)",
                                  isSelected: rowSelected,
                                  name: vm.inputChannelName(ch),
                                  meters: vm.meters,
                                  isRenaming: renamingChannel == ch,
                                  renameText: $renameText,
                                  rowHeight: sidebarRowHeight,
                                  onCommitRename: { commitRename() },
                                  isCurveVisible: vm.channelVisibility[ch] ?? true,
                                  onToggleCurve: { toggleCurve(ch) },
                                  nameWidth: sidebarNameWidth)
                            .onOptionClick { startRename(ch) }
                            .onTapGesture {
                                if renamingChannel != nil { commitRename() }
                                // Deselect on any row the current page already
                                // covers: with Link L/R on that page is the
                                // stereo pair's, so either row closes it.
                                if rowSelected {
                                    selection = .overview
                                    vm.updateSelection(to: nil)
                                } else {
                                    selection = .input(ch)
                                    vm.updateSelection(to: ch)
                                }
                            }
                            .contextMenu {
                                Button("Rename") { startRename(ch) }
                                    .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                                Divider()
                                Button("Copy Parameters") {
                                    vm.copyChannelParams(eqChannel: ch, name: vm.channelNames[ch])
                                }
                                Button("Paste Parameters") {
                                    vm.pasteChannelParams(eqChannel: ch)
                                }
                                .disabled(vm.channelClipboard == nil || !floor.allows(.other))   // -- WP SWF4 --, P1SW3
                            }
                    }
                }

                Section(header: Text("OUTPUTS")) {
                    ForEach(MatrixOutput.visible(for: vm.platformName, slotTypes: vm.outputSlotTypes).filter { vm.outputEnabled[$0.index] }, id: \.index) { out in
                        let eqCh = vm.eqChannel(forOutput: out.index)
                        OutputRow(output: out, isSelected: selection == .output(out.index),
                                  // -- BUG1 -- the label; a rename starts from the stored name
                                  name: vm.outputRowLabel(out.index),
                                  isMuted: vm.isOutputInactive(out.index),
                                  meters: vm.meters,
                                  isRenaming: renamingChannel == eqCh,
                                  renameText: $renameText,
                                  rowHeight: sidebarRowHeight,
                                  onCommitRename: { commitRename() },
                                  chIdx: eqCh,
                                  isCurveVisible: vm.channelVisibility[eqCh] ?? true,
                                  onToggleCurve: { toggleCurve(eqCh) },
                                  detail: vm.outputSlotName(out.index),
                                  nameWidth: sidebarNameWidth,
                                  pillAccessibleName: outputPillName(out))
                            .onOptionClick { startRename(eqCh) }
                            .onTapGesture {
                                if renamingChannel != nil { commitRename() }
                                if selection == .output(out.index) {
                                    selection = .overview
                                    vm.updateSelection(to: nil)
                                } else {
                                    selection = .output(out.index)
                                    vm.updateSelectionToOutput(out.index)
                                }
                            }
                            .contextMenu {
                                if vm.isDeviceConnected && vm.siggenSupported {
                                    Button("Identify") { vm.identifyOutput(out.index) }
                                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                                    Divider()
                                }
                                Button("Rename") { startRename(eqCh) }
                                    .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                                Divider()
                                Button("Copy Parameters") {
                                    vm.copyChannelParams(eqChannel: eqCh, name: vm.channelNames[eqCh])
                                }
                                Button("Paste Parameters") {
                                    vm.pasteChannelParams(eqChannel: eqCh)
                                }
                                .disabled(vm.channelClipboard == nil || !floor.allows(.other))   // -- WP SWF4 --, P1SW3
                            }
                    }
                }
            }
            .listStyle(.sidebar)
            // Faded rather than removed without a device: List animates
            // structural row changes with its own slide, and a pure crossfade
            // is wanted here.  Invisible rows are also inert, and they only
            // ever fade in fully formed - `isDeviceReady` waits for the
            // connect fetches, so the layout is final before it shows.
            .opacity(vm.isDeviceReady ? 1 : 0)
            .allowsHitTesting(vm.isDeviceReady)
            .animation(.easeInOut(duration: 0.3), value: vm.isDeviceReady)
            // Before the bottom inset, so the tour's spotlight covers the
            // scrolling channel rows and not the controls beneath them.
            .onboardingAnchor("basics.sidebar")
            .mask(
                VStack(spacing: 0) {
                    Color.black
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom)
                        .frame(height: 16)
                }
                .ignoresSafeArea(.all, edges: [.top, .bottom])
            )
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: 0) {
                    // Quick Access Icons
                    HStack(spacing: 0) {
                        SidebarIconButton(
                            icon: "slider.vertical.3",
                            isActive: matrixMixerController.isVisible,
                            tooltip: "Matrix Mixer",
                            action: { matrixMixerController.toggle() }
                        )
                        // Its own anchor, separate from the strip's: routing is
                        // a tour step in its own right, and the spotlight has
                        // to land on this button rather than all eight.
                        .onboardingAnchor("basics.routing")

                        SidebarIconButton(
                            icon: "headphones",
                            isActive: vm.crossfeedEnabled,
                            tooltip: "Headphone Crossfeed",
                            action: { vm.setCrossfeed(!vm.crossfeedEnabled) }
                        )
                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                        .onRightClick { crossfeedController.show(vm: vm) }

                        SidebarIconButton(
                            icon: "speaker.zzz",
                            isActive: vm.loudnessEnabled,
                            tooltip: "Loudness Compensation",
                            action: { vm.setLoudness(!vm.loudnessEnabled) }
                        )
                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                        .onRightClick { loudnessController.show(vm: vm) }

                        SidebarIconButton(
                            icon: "waveform.path.ecg",
                            isActive: vm.levellerEnabled,
                            tooltip: "Volume Leveller",
                            action: { vm.setLeveller(!vm.levellerEnabled) }
                        )
                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                        .onRightClick { levellerController.show(vm: vm) }

                        SidebarIconButton(
                            icon: "",
                            glyph: "\u{1D122}",   // bass clef (MUSICAL SYMBOL F CLEF)
                            glyphSize: 19,
                            glyphYOffset: 3,       // correct Apple Symbols bottom bearing
                            isActive: vm.psybassEnabled,
                            tooltip: "Psychoacoustic Bass",
                            action: { vm.setPsybass(!vm.psybassEnabled) }
                        )
                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                        .onRightClick { psybassController.show(vm: vm) }

                        SidebarIconButton(
                            icon: "info.circle",
                            isActive: statsController.isVisible,
                            tooltip: "Stats for Nerbs",
                            action: { statsController.toggle(transport: vm.transport) }
                        )

                        SidebarIconButton(
                            icon: "gearshape",
                            isActive: settingsWindowOpen,
                            tooltip: "Settings",
                            action: {
                                #if os(macOS)
                                if settingsWindowOpen, let w = NSApp.windows.first(where: {
                                    $0.identifier?.rawValue == "dspiSettings"
                                }) {
                                    w.close()
                                } else {
                                    openWindow(id: "settings")
                                    settingsWindowOpen = true
                                }
                                #else
                                // iOS: Settings is a sheet over the console.
                                settingsWindowOpen = true
                                #endif
                            }
                        )

                        SidebarIconButton(
                            icon: "xmark",
                            isActive: vm.bypass,
                            tooltip: "Bypass Master EQ",
                            action: { vm.setBypass(!vm.bypass) }
                        )
                        .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                    }
                    .padding(.vertical, 8)
                    .onboardingAnchor("basics.tools")

                    VStack(spacing: 0) {
                    Divider()

                    // Global Controls
                    VStack(alignment: .leading, spacing: 12) {
                        // Preset Picker
                        HStack {
                            Text("Preset").font(.caption2).foregroundColor(.secondary)
                            Spacer()
                            BorderlessPopUpButton(
                                items: Array(0..<10),
                                titleForItem: { slot in
                                    // Append "*" to the currently-selected slot's title when
                                    // unsaved changes are pending.  The popup face shows the
                                    // selected item's title verbatim, so the asterisk also
                                    // appears inside the picker box — exactly the dirty marker
                                    // we want.  Only the active slot ever gets the marker, so
                                    // when the user opens the menu they see at most one "*".
                                    let base = presetDropdownLabel(slot)
                                    return (slot == vm.activePresetSlot && vm.hasUnsavedChanges)
                                        ? "\(base)*"
                                        : base
                                },
                                selection: Binding(
                                    get: { vm.activePresetSlot },
                                    set: { slot in
                                        let previousSlot = vm.activePresetSlot
                                        print("[PRESET] set closure: slot=\(slot) previous=\(previousSlot) inFlight=\(presetSwitchInFlight) connected=\(vm.isDeviceConnected) unsaved=\(vm.hasUnsavedChanges)")
                                        guard slot != previousSlot else {
                                            print("[PRESET] SKIP: same slot")
                                            return
                                        }
                                        guard !presetSwitchInFlight else {
                                            print("[PRESET] SKIP: switch in flight")
                                            return
                                        }

                                        let showLoadFailed: (UInt8) -> Void = { status in
                                            vm.fetchPresetActive()
                                            NoticeCenter.main.post(AppNotices.presetLoad(status))   // -- WP P1SW3 -- was a modal (SWF7)
                                        }

                                        presetSwitchInFlight = true
                                        print("[PRESET] inFlight = true")
                                        vm.settleUnsavedChanges()   // -- WP SWF7 --
                                        if vm.isDeviceConnected && vm.hasUnsavedChanges {
                                            let diff = vm.computeDiff()
                                            let handle: (UnsavedChangesAction) -> Void = { action in
                                            switch action {
                                            case .save:
                                                DispatchQueue.global(qos: .userInitiated).async {
                                                    if vm.presetNames[previousSlot].isEmpty {
                                                        vm.setPresetName(slot: previousSlot, name: "Preset \(previousSlot + 1)")
                                                    }
                                                    let saveStatus = vm.savePreset(slot: previousSlot, confirmed: .savePreset())
                                                    guard saveStatus == PRESET_OK else {
                                                        DispatchQueue.main.async {
                                                            vm.activePresetSlot = previousSlot
                                                            NoticeCenter.main.post(AppNotices.preset(saveStatus, "Failed to save preset; the preset switch was stopped"))   // -- WP P1SW3 -- was a modal
                                                            presetSwitchInFlight = false
                                                        }
                                                        return
                                                    }
                                                    let loadStatus = vm.loadPreset(slot: slot)
                                                    DispatchQueue.main.async {
                                                        if loadStatus != PRESET_OK {
                                                            showLoadFailed(loadStatus)
                                                        }
                                                        presetSwitchInFlight = false
                                                    }
                                                }
                                            case .discard:
                                                DispatchQueue.global(qos: .userInitiated).async {
                                                    let status = vm.loadPreset(slot: slot)
                                                    DispatchQueue.main.async {
                                                        if status != PRESET_OK {
                                                            showLoadFailed(status)
                                                        }
                                                        presetSwitchInFlight = false
                                                    }
                                                }
                                            case .cancel:
                                                presetSwitchInFlight = false
                                                return
                                            }
                                            }
                                            #if os(macOS)
                                            handle(PresetAlerts.showUnsavedChangesAlert(diff: diff))
                                            #else
                                            PresetAlerts.showUnsavedChangesAlert(diff: diff, completion: handle)
                                            #endif
                                        } else {
                                            print("[PRESET] no unsaved changes, loading slot \(slot)")
                                            vm.activePresetSlot = slot
                                            DispatchQueue.global(qos: .userInitiated).async {
                                                let status = vm.loadPreset(slot: slot)
                                                print("[PRESET] loadPreset returned status=\(status)")
                                                DispatchQueue.main.async {
                                                    if status != PRESET_OK {
                                                        print("[PRESET] load FAILED status=\(status)")
                                                        showLoadFailed(status)
                                                    } else {
                                                        print("[PRESET] load OK, activeSlot=\(vm.activePresetSlot)")
                                                    }
                                                    presetSwitchInFlight = false
                                                    print("[PRESET] inFlight = false")
                                                }
                                            }
                                        }
                                    }
                                )
                            )
                            .frame(minWidth: 80)
                            .overlay(alignment: .trailing) {
                                Image(systemName: "chevron.up.chevron.down")
                                    .font(.system(size: 8, weight: .bold))
                                    .foregroundColor(.secondary)
                                    .padding(.trailing, 4)
                                    .allowsHitTesting(false)
                            }
                            .fixedSize()
                            // -- WP SWF4 -- No preset load or save on read-only firmware.
                            // -- WP P1SW3 -- The floor: recall stays under the installer's lock.
                            .opacity(vm.isDeviceConnected && floor.allows(.presetRecall) ? 1.0 : 0.4)
                            .allowsHitTesting(vm.isDeviceConnected && !presetSwitchInFlight && floor.allows(.presetRecall))
                            .floorHelp(!floor.allows(.presetRecall), floor.reason)
                        }
                        .onboardingAnchor("basics.presets")
                        .contextMenu {
                            // Wrapped in an Equatable view so SwiftUI's diff
                            // skips re-evaluating the menu body when only
                            // unrelated vm properties (e.g. peak meters) have
                            // changed.  Without this, the contextMenu's
                            // @ViewBuilder content is re-evaluated on every
                            // ContentView body re-render — meter polling
                            // produces a high-frequency rebuild storm that
                            // dismisses the open "Copy to…" submenu popover
                            // (AppKit then re-opens it on the still-hovered
                            // item, producing a visible open/close cycle).
                            PresetContextMenuItems(
                                activeSlot: vm.activePresetSlot,
                                slotLabels: (0..<10).map { presetLabel($0) },
                                isCurrentOccupied: vm.isPresetOccupied(vm.activePresetSlot),
                                anySlotOccupied: vm.presetOccupied != 0,
                                isStartupSpecified: vm.presetStartupMode == 0,
                                defaultSlot: vm.presetDefaultSlot,
                                isDeviceConnected: vm.isDeviceConnected,
                                onSave: { saveActivePreset(vm: vm) },
                                onRename: {
                                    presetRenameSlot = vm.activePresetSlot
                                    presetRenameText = vm.presetNames[vm.activePresetSlot]
                                    showPresetRename = true
                                },
                                onSetDefault: {
                                    let slot = vm.activePresetSlot
                                    vm.presetStartupMode = 0
                                    vm.presetDefaultSlot = slot
                                    DispatchQueue.global(qos: .userInitiated).async {
                                        vm.setPresetStartup(mode: 0, defaultSlot: slot)
                                    }
                                },
                                onCopyTo: { destSlot in copyActivePreset(to: destSlot) },
                                onClearActive: {
                                    showPresetClearConfirmation(vm: vm, slot: vm.activePresetSlot)
                                },
                                onClearAll: { showPresetClearAllConfirmation(vm: vm) }
                            )
                            .equatable()
                            .disabled(!floor.allows(.other))   // -- WP SWF4 --, P1SW3: the floor
                        }

                        // Input Source Picker (hidden if firmware doesn't support it)
                        if vm.inputSourceSupported {
                            HStack {
                                Text("Source").font(.caption2).foregroundColor(.secondary)
                                Spacer()
                                BorderlessPopUpButton(
                                    items: vm.inputSourceOptions,
                                    titleForItem: { vm.inputSourceTitle($0) },
                                    selection: Binding(
                                        get: { vm.inputSource },
                                        set: { source in
                                            guard source != vm.inputSource else { return }
                                            vm.setInputSource(source)
                                        }
                                    ),
                                    contextMenu: { sourceContextMenu() }
                                )
                                .frame(minWidth: 80)
                                .overlay(alignment: .trailing) {
                                    Image(systemName: "chevron.up.chevron.down")
                                        .font(.system(size: 8, weight: .bold))
                                        .foregroundColor(.secondary)
                                        .padding(.trailing, 4)
                                        .allowsHitTesting(false)
                                }
                                .fixedSize()
                                .opacity(vm.isDeviceConnected && floor.allows(.other) ? 1.0 : 0.4)   // -- WP SWF4 --, P1SW3
                                .allowsHitTesting(vm.isDeviceConnected && floor.allows(.other))
                                .floorHelp(!floor.allows(.other), floor.reason)
                            }
                        }

                        // Volume control — always drives the firmware's
                        // user volume directly via REQ_SET_USER_VOLUME (the
                        // same `audio_state.volume` field the OS volume
                        // slider writes via UAC1), regardless of input
                        // source.  We no longer touch the macOS system
                        // volume from here; instead the firmware reports
                        // OS-slider changes back as UAC1-tagged
                        // PARAM_CHANGED notifications, which keep this
                        // slider in sync (see applyNotifiedParamChange).
                        // Sidebar volume mode is user-selectable (Auto vs
                        // Master) via the popup-label selector inside each
                        // section; persisted in AppSettings.
                        switch SidebarVolumeMode(rawValue: settings.sidebarVolumeMode) ?? .auto {
                        // Only one is ever on screen, so both carry the same
                        // anchor: the tour points at whichever the user has.
                        case .master:
                            MasterModeSection(vm: vm)
                                .onboardingAnchor("basics.volume")
                        case .auto:
                            UserVolumeSection(vm: vm)
                                .onboardingAnchor("basics.volume")
                        }

                    }
                    .padding()
                    // ADDED GESTURE HERE FOR SIDEBAR CONTROLS
                    .onTapGesture {
                        if renamingChannel != nil { commitRename() }
                        #if os(macOS)
                NSApp.keyWindow?.makeFirstResponder(nil)
                #else
                UIApplication.shared.endEditing()
                #endif
                    }

                    Divider()

                    // System Status — Core 0 utilisation + master volume.
                    // Core 1 still appears in the Stats window; meters are
                    // inline in sidebar rows.
                    CpuSection(meters: vm.meters)
                    .padding()
                    }
                    .background(.ultraThinMaterial)
                }
            }
            .sidebarPaneFrame()
            // ADDED GESTURE FOR THE REST OF THE SIDEBAR
            .onTapGesture {
                if renamingChannel != nil { commitRename() }
                #if os(macOS)
                NSApp.keyWindow?.makeFirstResponder(nil)
                #else
                UIApplication.shared.endEditing()
                #endif
            }

            // MAIN CONTENT
            // Spacing is 2 because the graph block already ends in the 12pt
            // resize strip and the dashboard adds 4pt of its own top padding:
            // 12 + 2 + 4 gives the graph the same 18pt gap the cards use
            // between themselves.
            VStack(alignment: .leading, spacing: 2) {
                // Graph + connection status
                VStack(alignment: .leading, spacing: 0) {
                    // Header: graph title (or pop-out close button) on
                    // the left, connection status on the right.  Master
                    // volume now lives in the sidebar slider as a mode
                    // option, so it's no longer surfaced here.
                    HStack {
                        if graphWindowController.isVisible {
                            Button(action: { graphWindowController.hide() }) {
                                Image(systemName: "arrow.down.right.and.arrow.up.left")
                                    .font(.system(size: 10, weight: .medium))
                                    .foregroundColor(.secondary)
                            }
                            .buttonStyle(.plain)
                            .help("Close pop-out window")
                            .transition(.opacity)
                        } else {
                            Text("Filter Response").font(.headline)
                                .transition(.opacity)
                        }

                        Spacer()

                        ConnectionStatusIndicator(vm: vm)
                    }
                    .padding(.horizontal)
                    .padding(.top, 8)
                    // Popped out there is no graph or resize strip below the
                    // header, so it carries the gap to the content itself.
                    .padding(.bottom, graphWindowController.isVisible ? 14 : 16)

                    if !graphWindowController.isVisible {
                        BodePlotView(vm: vm)
                            .consoleGraphFrame(height: CGFloat(settings.graphHeight))
                            .padding(.horizontal)
                            .onboardingAnchor("basics.graph")
                            .transition(.scale(scale: 0.95, anchor: .top).combined(with: .opacity))
                        // No legend row here: the sidebar descriptor pills are
                        // the per-channel show/hide control in the main window,
                        // so the graph only needs the slim drag strip below it.
                        GraphResizeHandle(height: 12)
                    }
                }

                // Right Panel Content (Dynamic)
                VStack {
                    switch selection {
                    case .input(let ch):
                        // Edits mirror onto the other half of a linked pair.
                        let mirrorLink = vm.linkedPartner(of: ch)
                        VStack(spacing: 16) {
                            // The analyser taps inputs after their own PEQ, so
                            // these bars show the effect of the filters in the
                            // table below rather than the raw source.
                            SpectrumBarStrip(vm: vm, engine: vm.rta)
                                .padding(.horizontal)
                                .floorGated(.analyser)   // -- WP P1SW3 --

                            ReadOnlyFloorLine()   // -- WP P1SW3 --

                            InputChannelHeader(
                                channel: ch,
                                vm: vm,
                                onClearPEQ: {
                                    vm.clearChannelPEQ(ch)
                                    if let m = mirrorLink { vm.clearChannelPEQ(m) }
                                }
                            )
                            .padding(.horizontal)
                            .floorGated(.other)   // -- WP P1SW3 --

                            FilterListView(
                                bands: vm.channelData[ch] ?? [],
                                channelId: ch,
                                availableTypes: availableFilterTypes(vm: vm, includeLinkwitz: false),
                                bypassSupported: vm.firmwareSupportsBandBypass,
                                onUpdate: { band, params in
                                    vm.setFilter(ch: ch, band: band, p: params)
                                    if let m = mirrorLink {
                                        vm.setFilter(ch: m, band: band, p: params)
                                    }
                                },
                                onBypassToggle: { band, bypass in
                                    vm.setBandBypass(ch: ch, band: band, bypass: bypass)
                                    if let m = mirrorLink {
                                        vm.setBandBypass(ch: m, band: band, bypass: bypass)
                                    }
                                },
                                onClear: nil,  // handled by InputChannelHeader's Clear button
                                graphSelection: vm.peqSelection,
                                liveReadouts: vm.peqLive
                            )
                        }

                    case .output(let idx):
                        VStack(spacing: 16) {
                            // The output tap sits after gain and delay and
                            // before encoding, so this is exactly what the slot
                            // transmits.
                            SpectrumBarStrip(vm: vm, engine: vm.rta)
                                .padding(.horizontal)
                                .floorGated(.analyser)   // -- WP P1SW3 --

                            ReadOnlyFloorLine()   // -- WP P1SW3 --

                            OutputChannelDetail(vm: vm, outputIndex: idx,
                                                availableTypes: availableFilterTypes(vm: vm))
                        }

                    case .overview:
                        // `.never`, not `.hidden`: on macOS `.hidden` still
                        // brings the scroller back when a mouse is connected,
                        // and a legacy scroller steals ~15pt off the right edge,
                        // leaving the cards narrower than the graph above them.
                        ScrollView {
                            // -- WP P1SW3 -- the page scrolls; its controls follow the floor
                            DashboardOverview(vm: vm)
                                .floorGated(.other)
                        }
                        // -- WP P1SW3 -- the floor's line over the page: an inset, not a
                        // second child of the scroll view, so the dashboard keeps its
                        // one layout pass.
                        .safeAreaInset(edge: .top, spacing: 0) {
                            ReadOnlyFloorLine(verticalPadding: 4)
                        }
                        .scrollIndicators(.never)
                    }
                }
                // Greedy: the detail area soaks up all vertical slack, so
                // enlarging the graph above it shrinks this (scrollable) region
                // rather than growing the column.
                .frame(maxHeight: .infinity)
                // -- WP SW8 -- Firmware in another wire format: shown, not editable.
                // -- WP P1SW3 -- Each part follows the read-only floor
                // (`floorGated`), so what the lock allows stays live.
                .consoleDetailFrame()
            }
            // `idealHeight` pins the column's *reported* height to a constant,
            // so dragging the graph resize handle (which changes graphHeight)
            // no longer inflates the window - the extra graph height is taken
            // from the detail area above.  The sidebar (the other split pane)
            // is unaffected.  Still resizable between minHeight and maxHeight.
            .consoleColumnFrame()
            .background(Color(nsColor: .windowBackgroundColor).opacity(0.8))
            .onTapGesture {
                if renamingChannel != nil { commitRename() }
                #if os(macOS)
                NSApp.keyWindow?.makeFirstResponder(nil)
                #else
                UIApplication.shared.endEditing()
                #endif
            }
        }
        // Across the whole window rather than inside a pane: a device on the
        // wrong firmware is a broken state, not a per-page detail.
        //
        // An overlay, not a safe-area inset: an inset raises the view's minimum
        // height, and since the window is fixed-size AppKit grows it to suit -
        // the window jumped from 813pt to 928pt the moment the banner appeared.
        // An overlay contributes no size at all, so the banner draws over the
        // top of the pane and the window never moves.  That does mean it covers
        // the graph header while shown, which is what its Hide button is for.
        .overlay(alignment: .top) {
            // -- WP SW8 -- with the read-only notice under it
            VStack(spacing: 0) {
                #if os(macOS)
                FirmwareMismatchBanner(vm: vm) {
                    firmwareUpdateController.show(vm: vm)
                }
                #else
                // -- WP SW18 -- iOS: the banner has no Update button; it
                // points to a computer.
                FirmwareMismatchBanner(vm: vm)
                #endif
                ReadOnlyBanner(vm: vm)
                OfflineFileBadge(snaps: .shared)   // -- WP P1SW1 -- "File: <name>"
                LockBanner(installer: .shared)   // -- WP P1SW2 -- the locked user's front page line, Unlock
            }
        }
        // -- WP SWMAC1 -- What an undo or redo had to say, for a few seconds.
        // -- WP P1SW1 -- Above it the safe apply's countdown bar (Keep,
        // Revert), the A/B chip and the snapshots' notice line.
        .overlay(alignment: .bottom) {
            VStack(spacing: 0) {
                NoticeHost(center: .main)   // -- WP P1SW3 -- the window's notices (U5)
                SnapshotsOverlay(snaps: .shared)
                UndoNotice(undo: vm.undo)
            }
        }
        // -- WP P1SW3 -- the read-only floor for every control below
        .environment(\.readOnlyFloor, floor)
        #if os(macOS)
        // -- WP P1SW2 -- the unlock prompt and the mode switch (iOS: IOSRootView)
        .installerPrompts()
        #endif
        .navigationTitle("uConsole for DSPi")
        // The tour describes the filter table and the channel header, neither
        // of which exists on the overview the console opens on.  Selecting a
        // channel for those steps is the difference between a coach mark that
        // points at something and one that points at empty space.
        .onChange(of: onboarding.basicsTourStep?.id) { _ in
            syncTourWindows()
            guard let step = onboarding.basicsTourStep, step.needsChannelDetail,
                  vm.numMatrixInputs > 0 else { return }
            if case .input = selection { return }
            selection = .input(0)
        }
        // Also on the running flag, so finishing or skipping from inside the
        // Matrix Mixer puts the window away.
        .onChange(of: onboarding.basicsTourRunning) { _ in syncTourWindows() }
        .onChange(of: vm.outputEnabled) { _ in
            // If the selected output was disabled, fall back to overview
            if case .output(let idx) = selection, !vm.outputEnabled[idx] {
                selection = .overview
                vm.updateSelection(to: nil)
            }
        }
        .onChange(of: vm.isDeviceConnected) { connected in
            // The sidebar hides its channel rows without a device, so a
            // selection left on a channel page would have no row to close it.
            if !connected, selection != .overview {
                withAnimation(.easeInOut(duration: 0.3)) {
                    selection = .overview
                    vm.updateSelection(to: nil)
                }
            }
        }
        .sheet(isPresented: $showSourceRename) {
            VStack(spacing: 16) {
                Text("Rename Source").font(.headline)
                TextField(vm.builtInInputSourceTitle(sourceRenameTarget), text: $sourceRenameText)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 200)
                    .onChange(of: sourceRenameText) { _, new in
                        let clean = DSPViewModel.sanitizeInputSourceName(new)
                        if clean != new { sourceRenameText = clean }
                    }
                Text("Leave empty to use \u{201C}\(vm.builtInInputSourceTitle(sourceRenameTarget))\u{201D}.")
                    .font(.caption)
                    .foregroundColor(.secondary)
                HStack {
                    Button("Cancel") { showSourceRename = false }
                        .keyboardShortcut(.cancelAction)
                    Button("Rename") {
                        commitSourceRename(sourceRenameTarget, sourceRenameText)
                        showSourceRename = false
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding()
            .frame(width: 280)
        }
        .sheet(isPresented: $showPresetRename) {
            VStack(spacing: 16) {
                Text("Rename Preset").font(.headline)
                TextField("Name", text: $presetRenameText)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 200)
                HStack {
                    Button("Cancel") { showPresetRename = false }
                        .keyboardShortcut(.cancelAction)
                    Button("Rename") {
                        let name = presetRenameText.trimmingCharacters(in: .whitespaces)
                        DispatchQueue.global(qos: .userInitiated).async {
                            vm.setPresetName(slot: presetRenameSlot, name: name)
                        }
                        showPresetRename = false
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding()
            .frame(width: 280)
        }
        #if os(macOS)
        .onAppear {
            DispatchQueue.main.async {
                if let window = NSApp.keyWindow {
                    window.isMovableByWindowBackground = true
                    window.setContentSize(NSSize(width: 950, height: 813))
                    window.styleMask.remove(.resizable)
                }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
            if let window = note.object as? NSWindow,
               window.identifier?.rawValue == "dspiSettings" {
                settingsWindowOpen = false
            }
        }
        #else
        .sheet(isPresented: $settingsWindowOpen) {
            IOSSettingsSheet()
        }
        // `-SelectChannel in1|out3...` opens a channel page once the device is
        // ready: a hook for Simulator screenshots and UI tests.
        .onReceive(vm.$isDeviceReady) { ready in
            guard ready, !ContentView.didApplyLaunchSelection,
                  let target = UserDefaults.standard.string(forKey: "SelectChannel")?.lowercased(),
                  let number = Int(target.drop { !$0.isNumber }), number >= 1 else { return }
            ContentView.didApplyLaunchSelection = true
            if target.hasPrefix("in"), number <= vm.numMatrixInputs {
                selection = .input(number - 1)
                vm.updateSelection(to: number - 1)
            } else if target.hasPrefix("out"), number <= vm.numOutputChannels {
                selection = .output(number - 1)
                vm.updateSelectionToOutput(number - 1)
            }
        }
        #endif
    }

}

// MARK: - Preview Support

extension DSPViewModel {
    /// Creates a preview-safe view model with mock data (no USB connection)
    static var preview: DSPViewModel {
        #if os(macOS)
        let vm = DSPViewModel(transport: USBDevice())
        #else
        let vm = DSPViewModel(transport: FakeTransport(autoSelect: false))
        #endif
        vm.isDeviceConnected = true
        vm.preampDB = [-3.0, -3.0]
        vm.meters.status = SystemStatus(peaks: [0.6, 0.55, 0.4, 0.35, 0.25], cpu0: 42, cpu1: 38)

        // Add some sample filter data for Master L
        vm.channelData[Channel.masterLeft.rawValue] = [
            FilterParams(type: .peaking, freq: 100, q: 0.7, gain: -5.0),
            FilterParams(type: .peaking, freq: 400, q: 1.0, gain: 3.0),
            FilterParams(type: .highShelf, freq: 8000, q: 0.7, gain: -2.0),
            FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams()
        ]

        // Add some sample filter data for Master R
        vm.channelData[Channel.masterRight.rawValue] = [
            FilterParams(type: .peaking, freq: 100, q: 0.7, gain: -5.0),
            FilterParams(type: .peaking, freq: 400, q: 1.0, gain: 3.0),
            FilterParams(type: .highShelf, freq: 8000, q: 0.7, gain: -2.0),
            FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams(), FilterParams()
        ]

        // Enable crossover tab in preview so the new UI is visible.
        vm.firmwareSupportsCrossover = true

        return vm
    }
}

// MARK: - Output Channel Detail
//
// Right-pane content for an output channel.  Shared output-settings card on
// top, then a tab bar that switches the band list below between PEQ and
// crossover bands.  Both tabs render the same FilterListView — only the
// `bands`, `availableTypes`, and update callbacks differ.

enum OutputChannelTab: Hashable {
    case peq
    case crossover
}

/// PEQ-only filter types (everything in `FilterType.allCases` except the
/// crossover entries) — kept here as a stand-alone helper because the
/// crossover tab needs the complement.
fileprivate func availableCrossoverTypes(vm: DSPViewModel) -> [FilterType] {
    // FLAT is included so the user can disarm a band by picking "Off".
    [.flat] + FilterType.allCases.filter { $0.isCrossover }
}

struct OutputChannelDetail: View {
    @ObservedObject var vm: DSPViewModel
    let outputIndex: Int
    let availableTypes: [FilterType]
    @State private var tab: OutputChannelTab = .peq

    private var eqChannel: Int { vm.eqChannel(forOutput: outputIndex) }
    private var crossoverSupported: Bool { vm.firmwareSupportsCrossover }

    private var filterListTabs: [FilterListTab] {
        guard crossoverSupported else { return [] }
        return [
            FilterListTab(title: "PEQ",       isSelected: tab == .peq)       { tab = .peq },
            FilterListTab(title: "XO",        isSelected: tab == .crossover) { tab = .crossover },
        ]
    }

    var body: some View {
        VStack(spacing: 16) {
            ChannelSettingsView(
                vm: vm,
                outputIndex: outputIndex,
                gainDB: Binding(
                    get: { vm.outputGainDB[outputIndex] },
                    set: { vm.setOutputGain(output: outputIndex, db: $0) }
                ),
                delayMS: Binding(
                    get: { vm.outputDelayMS[outputIndex] },
                    set: { vm.setOutputDelay(output: outputIndex, ms: $0) }
                ),
                isMuted: Binding(
                    get: { vm.outputMuted[outputIndex] },
                    set: { vm.setOutputMute(output: outputIndex, muted: $0) }
                ),
                maxDelay: vm.maxOutputDelayMS,   // -- WP SWF8 -- the core's line
                onGainDrag: { vm.sendOutputGainToDevice(output: outputIndex, db: $0) },
                onDelayDrag: { vm.sendOutputDelayToDevice(output: outputIndex, ms: $0) }
            )
            .padding(.horizontal)

            switch tab {
            case .peq:
                FilterListView(
                    bands: vm.channelData[eqChannel] ?? [],
                    channelId: eqChannel,
                    availableTypes: availableTypes,
                    bypassSupported: vm.firmwareSupportsBandBypass,
                    tabs: filterListTabs,
                    onUpdate: { band, params in
                        vm.setFilter(ch: eqChannel, band: band, p: params)
                    },
                    onBypassToggle: { band, bypass in
                        vm.setBandBypass(ch: eqChannel, band: band, bypass: bypass)
                    },
                    onClear: { vm.clearPEQBands(ch: eqChannel) },
                    graphSelection: vm.peqSelection,
                    liveReadouts: vm.peqLive
                )
            case .crossover:
                FilterListView(
                    bands: vm.xoverData[eqChannel] ?? [],
                    channelId: eqChannel,
                    availableTypes: availableCrossoverTypes(vm: vm),
                    bypassSupported: vm.firmwareSupportsBandBypass,
                    tabs: filterListTabs,
                    isCrossoverMode: true,
                    onUpdate: { localBand, params in
                        vm.setCrossoverBand(ch: eqChannel, localBand: localBand, p: params)
                    },
                    onBypassToggle: { localBand, bypass in
                        vm.setCrossoverBandBypass(ch: eqChannel, localBand: localBand, bypass: bypass)
                    },
                    onClear: { vm.clearCrossoverBands(ch: eqChannel) }
                )
            }
        }
        .onAppear { vm.crossoverTabShown = tab == .crossover }
        .onDisappear { vm.crossoverTabShown = false }
        .onChange(of: tab) { vm.crossoverTabShown = $0 == .crossover }
        .onChange(of: crossoverSupported) { supported in
            // If we lose crossover support (e.g. user switches to a pre-V11
            // device), bounce back to the PEQ tab so we don't show an
            // unreachable selection.
            if !supported && tab == .crossover { tab = .peq }
        }
    }
}

// MARK: - User Volume Section
//
// Drives REQ_SET_USER_VOLUME (0xDA) — the vendor-channel access to the
// same `audio_state.volume` field the OS volume slider writes via UAC1.
// This is the sole "Auto" volume control for every input source: the
// firmware applies the value to vol_mul + the loudness coefficient
// pointer regardless of source, so equal-loudness compensation tracks
// changes correctly.
//
// We never touch the macOS system volume.  Instead, OS volume changes
// flow OS → device (over UAC1) → UAC1-tagged PARAM_CHANGED notification
// → vm.userVolumeDB, so this slider stays synced with the OS slider.
// The reverse (slider → OS volume) isn't possible under UAC1.
struct UserVolumeSection: View {
    @ObservedObject var vm: DSPViewModel
    @ObservedObject private var settings = AppSettings.shared
    @Environment(\.readOnlyFloor) private var floor   // -- WP P1SW3 --

    // Slider position is a "scalar" 0...1 mapped to dB over [minDB, 0]
    // via a square-root power taper: more travel is given to the loud
    // (top) region for fine control, with the quiet region compressed
    // toward the bottom.  At scalar 0 the firmware gets the floor
    // (minDB) since user_volume is unmuted-only.
    @State private var localScalar: Double = 1.0
    @State private var isDragging = false
    @State private var dragMark = CoreDragMark()   // -- WP SW9 --

    private static let minDB: Float = USER_VOLUME_MIN_DB   // -60

    private static func scalarToDB(_ s: Double) -> Float {
        if s <= 0 { return minDB }
        let span = Double(-minDB)               // 60
        return Float(Double(minDB) + span * sqrt(min(1, s)))
    }

    private static func dbToScalar(_ db: Float) -> Double {
        if db <= minDB { return 0 }
        let span = Double(-minDB)               // 60
        let frac = Double(db - minDB) / span    // 0...1
        return min(1, frac * frac)
    }

    private static func displayString(scalar: Double, db: Float) -> String {
        return String(format: "%.1f dB", db)
    }

    var body: some View {
        VStack(spacing: 4) {
            HStack {
                VolumeModeSelector(modeRaw: $settings.sidebarVolumeMode,
                                   inputSource: vm.inputSource)
                Spacer()
                Text(Self.displayString(scalar: localScalar,
                                        db: Self.scalarToDB(localScalar)))
                    .font(.system(.body).monospacedDigit())
                    .foregroundColor(.primary)
            }
            Slider(value: $localScalar, in: 0...1) { editing in
                dragMark.set(editing)   // -- WP SWMAC1 --, SW9
                isDragging = editing
                if !editing { vm.setUserVolume(Self.scalarToDB(localScalar)) }
            }
            .endsCoreDrag(dragMark)   // -- WP SW9 -- ended if the view goes mid-drag
            .controlSize(.small)
            .onChange(of: localScalar) { val in
                if isDragging { vm.sendUserVolumeToDevice(Self.scalarToDB(val)) }
            }
            .onAppear { localScalar = Self.dbToScalar(vm.userVolumeDB) }
            .onChange(of: vm.userVolumeDB) { val in
                if !isDragging { localScalar = Self.dbToScalar(val) }
            }
            .onRightClick { guard floor.allows(.volume) else { return }; localScalar = 1; vm.setUserVolume(0) }
            .floorGated(.volume)   // -- WP SWF4 --, P1SW3: live under the installer's lock
        }
    }
}

// MARK: - Master Mode Section
//
// Sidebar control bound directly to vm.masterVolumeDB.  Distinguished
// from Host / User mode by:
//   1. The popup label reads "Master Volume ▾" (the selector adapts).
//   2. The slider track is tinted red.
//   3. The slider taper is the firmware's piecewise-linear master taper
//      (see MasterVolumeTaper) — different range from host/user (-128 to
//      0 dB vs -60 to 0 dB) and different step granularity per region.
// Live drag uses sendMasterVolumeToDevice; commit-on-release uses
// setMasterVolume.  Right-click resets to 0 dB.  At slider bottom the
// readout shows "−∞" (mute).
struct MasterModeSection: View {
    @ObservedObject var vm: DSPViewModel
    @ObservedObject private var settings = AppSettings.shared
    @Environment(\.readOnlyFloor) private var floor   // -- WP P1SW3 --

    @State private var localDB: Float = 0
    @State private var isDragging = false
    @State private var dragMark = CoreDragMark()   // -- WP SW9 --

    private var sliderPos: Double {
        Double(MasterVolumeTaper.dbToSlider(localDB))
    }

    private var displayString: String {
        if localDB <= -128 { return "−∞" }
        return MasterVolumeTaper.format(localDB) + " dB"
    }

    var body: some View {
        VStack(spacing: 4) {
            HStack {
                VolumeModeSelector(modeRaw: $settings.sidebarVolumeMode,
                                   inputSource: vm.inputSource)
                Spacer()
                Text(displayString)
                    .font(.system(.body).monospacedDigit())
                    .foregroundColor(.primary)
            }
            Slider(value: Binding(
                get: { sliderPos },
                set: { localDB = MasterVolumeTaper.sliderToDB(Float($0)) }
            ), in: 0...1) { editing in
                dragMark.set(editing)   // -- WP SWMAC1 --, SW9
                isDragging = editing
                if !editing { vm.setMasterVolume(localDB) }
            }
            .endsCoreDrag(dragMark)   // -- WP SW9 -- ended if the view goes mid-drag
            .controlSize(.small)
            .tint(.red)
            .onChange(of: localDB) { val in
                if isDragging { vm.sendMasterVolumeToDevice(val) }
            }
            .onAppear { localDB = vm.masterVolumeDB }
            .onChange(of: vm.masterVolumeDB) { val in
                if !isDragging { localDB = val }
            }
            .onRightClick { guard floor.allows(.volume) else { return }; localDB = 0; vm.setMasterVolume(0) }
            .floorGated(.volume)   // -- WP SWF4 --, P1SW3: live under the installer's lock
        }
    }
}

#Preview("Dashboard") {
    ContentView(vm: .preview)
        .environmentObject(MatrixMixerWindowController())
        .environmentObject(LoudnessWindowController())
        .environmentObject(CrossfeedWindowController())
        .environmentObject(StatsWindowController())
        .environmentObject(GraphWindowController())
        .environmentObject(VolumeLevellerWindowController())
        .environmentObject(PsychoacousticBassWindowController())
        #if os(macOS)
        .environmentObject(FirmwareUpdateWindowController())
        #endif
        .environmentObject(OnboardingCoordinator.shared)
        .frame(height: 790)
}

#Preview("Channel Selected") {
    ContentView(vm: .preview)
        .environmentObject(MatrixMixerWindowController())
        .environmentObject(LoudnessWindowController())
        .environmentObject(CrossfeedWindowController())
        .environmentObject(StatsWindowController())
        .environmentObject(GraphWindowController())
        .environmentObject(VolumeLevellerWindowController())
        .environmentObject(PsychoacousticBassWindowController())
        #if os(macOS)
        .environmentObject(FirmwareUpdateWindowController())
        #endif
        .environmentObject(OnboardingCoordinator.shared)
        .frame(width: 1000, height: 780)
}

// MARK: - Preset Context Menu Items
//
// Equatable view holding the contents of the preset-picker context menu.
// Conforms to Equatable on the *data* fields only — the closures are excluded
// from `==` (they're not Equatable in Swift, but they all dispatch through the
// same long-lived `vm`/`@State`, so identity-of-closure doesn't actually
// affect what the menu does).
//
// Why Equatable matters here: SwiftUI re-evaluates a contextMenu's
// @ViewBuilder on every parent body re-render.  In ContentView, those happen
// at meter-polling rate (~60 Hz).  Without `.equatable()`, every re-render
// rebuilds the underlying NSMenu and dismisses any open submenu — AppKit then
// auto-reopens it on the still-hovered "Copy to…" item, producing a visible
// open/close cycle.  With `.equatable()`, SwiftUI compares old/new instances,
// finds the data fields unchanged, and skips rebuilding the menu entirely.
private struct PresetContextMenuItems: View, Equatable {
    let activeSlot: Int
    let slotLabels: [String]              // index 0..9 → "1: Living Room"
    let isCurrentOccupied: Bool
    let anySlotOccupied: Bool
    let isStartupSpecified: Bool          // presetStartupMode == 0
    let defaultSlot: Int
    let isDeviceConnected: Bool

    let onSave: () -> Void
    let onRename: () -> Void
    let onSetDefault: () -> Void
    let onCopyTo: (Int) -> Void
    let onClearActive: () -> Void
    let onClearAll: () -> Void

    static func == (lhs: PresetContextMenuItems, rhs: PresetContextMenuItems) -> Bool {
        lhs.activeSlot == rhs.activeSlot
            && lhs.slotLabels == rhs.slotLabels
            && lhs.isCurrentOccupied == rhs.isCurrentOccupied
            && lhs.anySlotOccupied == rhs.anySlotOccupied
            && lhs.isStartupSpecified == rhs.isStartupSpecified
            && lhs.defaultSlot == rhs.defaultSlot
            && lhs.isDeviceConnected == rhs.isDeviceConnected
    }

    var body: some View {
        Group {
            Button("Save", action: onSave)
            Button("Rename…", action: onRename)
            Button("Set as Default", action: onSetDefault)
                .disabled(isStartupSpecified && defaultSlot == activeSlot)

            Menu("Copy to…") {
                ForEach(0..<10, id: \.self) { destSlot in
                    if destSlot != activeSlot {
                        Button(slotLabels[destSlot]) { onCopyTo(destSlot) }
                    }
                }
            }
            .disabled(!isDeviceConnected)

            if isCurrentOccupied {
                Divider()
                Button("Clear \"\(slotLabels[activeSlot])\"…", role: .destructive, action: onClearActive)
            }
            if anySlotOccupied {
                Divider()
                Button("Clear All Slots…", role: .destructive, action: onClearAll)
            }
        }
    }
}

#if os(macOS)
// Frames that iOS sizes by size class instead (IOSApp.swift).
private extension View {
    /// The sidebar pane's width.  iOS: the full width when a compact window
    /// shows one pane at a time.
    func sidebarPaneFrame() -> some View { frame(minWidth: 220, maxWidth: 260) }

    /// The response graph's height.  iOS: capped in a short window (a phone
    /// in landscape).
    func consoleGraphFrame(height: CGFloat) -> some View { frame(height: height) }

    /// The channel page or dashboard below the graph.  iOS: scrolls sideways
    /// on a screen narrower than its tables.
    func consoleDetailFrame() -> some View { self }

    /// The console column.  iOS: no minimum height, which a phone in
    /// landscape does not have.
    func consoleColumnFrame() -> some View {
        frame(maxWidth: .infinity, minHeight: 500, idealHeight: 620, maxHeight: .infinity, alignment: .top)
    }
}
#endif
