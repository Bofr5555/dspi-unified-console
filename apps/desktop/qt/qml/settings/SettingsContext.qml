import QtQuick 2.15

// Shared state for the Settings pages: the app and window, navigation, and
// the draft of board-wide settings that are only written on Save.
QtObject {
    id: ctx
    property var app                 // main window (graph preferences live there)
    property var window              // the Settings window
    signal navigateRequested(string pageId)
    function navigate(pageId) { navigateRequested(pageId) }

    // ── Global Parameters draft ──
    // Edited freely; nothing reaches the device until save().
    property int startupMode: 0          // 0 = specified default, 1 = last used
    property int defaultSlot: 0
    property int masterVolumeMode: 0     // 0 = independent, 1 = with preset
    property int outputConfigMode: 1     // 0 = independent, 1 = with preset
    // External DAC mute (Global Parameters), sent on Save as on macOS
    property bool dacEnabled: false
    property bool dacActiveLow: true
    property int dacPin: 0xFF
    property int dacHoldMs: 5
    property int dacReleaseMs: 0
    property bool edited: false

    readonly property bool dacDirty: {
        var h = bridge.hardware
        return dacEnabled !== h.dacMuteEnabled || (dacEnabled && (dacActiveLow !== h.dacMuteActiveLow
            || dacPin !== h.dacMutePin || dacHoldMs !== h.dacMuteHoldMs || dacReleaseMs !== h.dacMuteReleaseMs))
    }
    // -- WP QD4 -- Source and device names, whose drafts the controller holds
    readonly property bool namesDirty: bridge.connected && deviceNames.dirty
    // -- WP QD5 -- The latency, whose draft the controller holds
    readonly property bool latencyDirty: bridge.connected && outputLatency.dirty
    // -- WP QD31 -- The bridge's settings differ from the device's
    readonly property bool settingsDiffer: startupMode !== bridge.presetStartupMode
        || (startupMode === 0 && defaultSlot !== bridge.presetDefaultSlot)
        || masterVolumeMode !== bridge.masterVolumeMode
        || outputConfigMode !== bridge.outputConfigMode
        || dacDirty
    readonly property bool draftDirty: namesDirty || latencyDirty || (edited && bridge.connected && settingsDiffer)
    // Hardware edits in independent mode are RAM-only until the output
    // config is saved; they share the save bar (but cannot be reverted)
    readonly property bool hardwareDirty: bridge.connected && bridge.hardwareUnsaved
    // Control surfaces: applied edits not yet saved to flash
    readonly property bool csDirty: bridge.connected && controlSurfaces.unsaved
    readonly property bool dirty: draftDirty || hardwareDirty || csDirty
    // -- WP QD1 --
    // Why the device refused the last Save or Revert; the save bar shows it. load() leaves
    // it: the bridge's state changes reach load() while a refused save stands.
    property string saveError: ""
    onDirtyChanged: if (!dirty) saveError = ""

    function load() {
        startupMode = bridge.presetStartupMode
        defaultSlot = bridge.presetDefaultSlot
        masterVolumeMode = bridge.masterVolumeMode
        outputConfigMode = bridge.outputConfigMode
        var h = bridge.hardware
        dacEnabled = h.dacMuteEnabled
        dacActiveLow = h.dacMuteActiveLow
        dacPin = h.dacMutePin
        dacHoldMs = h.dacMuteHoldMs
        dacReleaseMs = h.dacMuteReleaseMs
        edited = false
    }

    function edit(name, value) {
        saveError = ""   // -- WP QD1 --
        ctx[name] = value
        edited = true
    }

    // -- WP QD1 --
    // Write the control surfaces to flash; false (with the reason) if refused
    function saveControlSurfaces() {
        if (!bridge.writable) return false   // -- WP QD6 --
        var st = controlSurfaces.save()
        saveError = st !== 0 ? "Not saved: " + controlSurfaces.statusMessage(st) : ""
        return st === 0
    }
    function revertControlSurfaces() {
        if (!bridge.writable) return false   // -- WP QD6 --
        var st = controlSurfaces.revert()
        saveError = st !== 0 ? "Not reverted: " + controlSurfaces.statusMessage(st) : ""
        return st === 0
    }

    // -- WP QD31 -- A PRESET_* status as the reason an output config save failed
    function outputConfigStatusText(code) {
        switch (code) {
        case 4: return "the device could not write its flash"
        case 255: return "the device did not respond"
        default: return "the device reported error " + code
        }
    }

    function save() {
        saveError = ""   // -- WP QD1 --
        if (!bridge.writable) return   // -- WP QD6 --
        // -- WP QD31 -- Stop at the first refusal, as for the control surfaces below: the draft
        // stays, with the reason on the bar. What went through already now matches the device.
        if (startupMode !== bridge.presetStartupMode || defaultSlot !== bridge.presetDefaultSlot) {
            if (!bridge.setPresetStartup(startupMode, defaultSlot)) {
                saveError = "Not saved: the device did not accept the startup preset"
                return
            }
        }
        if (masterVolumeMode !== bridge.masterVolumeMode && !bridge.setMasterVolumeMode(masterVolumeMode)) {
            saveError = "Not saved: the device did not accept the master volume mode"
            return
        }
        if (outputConfigMode !== bridge.outputConfigMode && !bridge.setOutputConfigMode(outputConfigMode)) {
            saveError = "Not saved: the device did not accept the output config mode"
            return
        }
        var dacSent = false
        if (dacDirty) {
            if (!bridge.setDacMute(dacEnabled, dacActiveLow, dacPin, dacHoldMs, dacReleaseMs))
                return   // dacWatch gave the reason
            dacSent = true
        }
        if (hardwareDirty) {
            var st = bridge.saveOutputConfig()
            if (st !== 0) {
                saveError = "Not saved: " + outputConfigStatusText(st)
                return
            }
        }
        // -- WP QD1 -- A refused save must never look saved: the draft stays
        if (csDirty && !saveControlSurfaces())
            return
        // -- WP QD4 -- The controller keeps a refused name's draft; see namesWatch
        if (namesDirty)
            deviceNames.save()
        // -- WP QD5 -- Last, as on the Mac: it restarts the outputs. See latencyWatch.
        if (latencyDirty)
            outputLatency.save()
        // -- WP QD31 -- A DAC mute sent isn't saved yet: dacWatch ends the draft on the read-back
        if (!dacSent) edited = false
    }

    // ── Pins ──

    // Who holds a GPIO ("" = free), as the firmware reserves it
    function ownerOf(pin) {
        var owners = bridge.pinOwners()
        for (var i = 0; i < owners.length; i++)
            if (owners[i].pin === pin) return owners[i].owner
        return ""
    }
    // Picker options for a GPIO: the wirable pins nothing else holds, plus
    // `pin` itself. accept(p) narrows the list; owners named in `sharable`
    // don't count as holding their pin.
    function pinOptions(pin, accept, sharable, allowUnset, unsetText) {
        var owners = bridge.pinOwners(), taken = {}
        for (var i = 0; i < owners.length; i++)
            if (!sharable || sharable.indexOf(owners[i].owner) < 0) taken[owners[i].pin] = true
        var list = []
        if (allowUnset) list.push({ text: unsetText || "Not set", value: 0xFF })
        var pins = bridge.validPins
        for (var k = 0; k < pins.length; k++) {
            var p = pins[k]
            if (p !== pin && (taken[p] || (accept && !accept(p)))) continue
            list.push({ text: "GPIO " + p, value: p })
        }
        if (pin !== 0xFF && pins.indexOf(pin) < 0) list.push({ text: "GPIO " + pin, value: pin })
        return list
    }
    function optionIndex(options, value) {
        for (var i = 0; i < options.length; i++) if (options[i].value === value) return i
        return -1
    }

    // A PIN_CONFIG_* status as a sentence ("" for success)
    function statusText(code, pin) {
        switch (code) {
        case 0: return ""
        case 1: return pin !== undefined ? "GPIO " + pin + " can't be used for this." : "That pin can't be used for this."
        case 2: {
            var who = pin !== undefined ? ownerOf(pin) : ""
            return pin !== undefined ? "GPIO " + pin + " is already assigned" + (who !== "" ? " to " + who : "") + "."
                                     : "That pin is already in use."
        }
        case 3: return "This isn't supported on this device."
        case 4: return "Turn this off before changing it."
        case 5: return "That value is out of range."
        case 255: return "The device did not respond."
        default: return "Error " + code + "."
        }
    }

    function revert() {
        saveError = ""   // -- WP QD1 --
        // -- WP QD1 -- A refused revert must not look reverted: the draft stays
        if (csDirty && !revertControlSurfaces()) return
        deviceNames.revert()   // -- WP QD4 --
        outputLatency.revert()   // -- WP QD5 --
        load()
    }

    // Follow the device while there is no draft
    property Connections bridgeWatch: Connections {
        target: bridge
        function onStateChanged() { if (!ctx.edited) ctx.load() }
    }
    // -- WP QD31 -- The device's verdict on a DAC mute, which it applies in its main loop
    property Connections dacWatch: Connections {
        target: bridge
        function onDacMuteApplied(accepted) {
            if (!accepted) {
                if (ctx.dirty) ctx.saveError = "Not saved: the device did not keep the DAC mute settings"
            } else if (ctx.edited && !ctx.settingsDiffer) {
                ctx.edited = false
                ctx.load()
            }
        }
    }
    // -- WP QD4 --
    property Connections namesWatch: Connections {
        target: deviceNames
        function onSaveFailed(reason) { ctx.saveError = "Not saved: " + reason }
    }
    // -- WP QD5 --
    property Connections latencyWatch: Connections {
        target: outputLatency
        property string shownError: ""
        function onSaveFailed(reason) { shownError = "Not saved: " + reason; ctx.saveError = shownError }
        // Another draft may keep the save bar up; its own refusal stays
        function onDraftSettled() { if (ctx.saveError === shownError) ctx.saveError = "" }
    }
    Component.onCompleted: load()
}
