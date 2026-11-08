import QtQuick 2.15
import QtQuick.Controls 2.15
import "../"
import "../../components"

// Board-wide settings. Edits are a draft until Save in the save bar.
SettingsPage {
    id: page
    title: qsTr("Global Parameters")
    subtitle: qsTr("Board-wide settings that don't belong to any preset. Changes are held until you click Save.")

    readonly property var hw: bridge.hardware
    property string dacStatus: ""
    property bool dacStatusError: false
    Connections {
        target: bridge
        function onDacMuteApplied(accepted) {
            page.dacStatusError = !accepted
            page.dacStatus = accepted ? qsTr("External mute configuration saved.")
                                      : qsTr("The device rejected the mute configuration. The pin may be in use by another function.")
        }
    }
    function msOptions(list, current) {
        var l = list.slice()
        if (l.indexOf(current) < 0) { l.push(current); l.sort(function(a, b) { return a - b }) }
        return l.map(function(v) { return { text: qsTr("%1 ms").arg(v), value: v } })
    }

    readonly property var presetOptions: {
        bridge.presetOccupied
        var o = []
        for (var i = 0; i < 10; i++) {
            var name = bridge.presetName(i)
            var label = bridge.isPresetOccupied(i) ? (name === "" ? qsTr("Preset %1").arg(i + 1) : name) : qsTr("Empty")
            o.push({ text: (i + 1) + ". " + label, value: i })
        }
        return o
    }

    // -- WP QD4 --
    SettingsSection {
        visible: deviceNames.deviceNameSupported
        title: qsTr("Device Name")
        enabled: bridge.connected
        footnote: qsTr("Stored on the device, so it keeps this name wherever it is connected. Shown on its front panel and by DSPi Link hubs.")
        SettingsNameRow {
            title: qsTr("Name")
            detail: bridge.selectedSerial !== "" ? qsTr("Serial %1").arg(bridge.selectedSerial) : ""
            fieldName: "deviceNameField"
            clearTip: qsTr("Remove the name")
            value: deviceNames.deviceText
            pending: { deviceNames.deviceText; return deviceNames.devicePending() }
            onEdited: function (text) { deviceNames.editDevice(text) }
        }
    }

    SettingsSection {
        title: qsTr("Startup Preset")
        enabled: bridge.connected
        SettingsChoiceRow {
            title: qsTr("At Power-On")
            options: [{ text: qsTr("Specified Default"), value: 0 }, { text: qsTr("Last Used"), value: 1 }]
            value: ctx ? ctx.startupMode : 0
            onChosen: ctx.edit("startupMode", value)
        }
        SettingsChoiceRow {
            title: qsTr("Default Preset")
            enabled: ctx ? ctx.startupMode === 0 : true
            options: page.presetOptions
            value: ctx ? ctx.defaultSlot : 0
            onChosen: ctx.edit("defaultSlot", value)
        }
    }

    SettingsSection {
        title: qsTr("Master Volume")
        enabled: bridge.connected
        footnote: ctx && ctx.masterVolumeMode === 0
                  ? qsTr("Master volume is stored once on the board and applied at power-on. Loading a preset never changes it. Save it from the menu with Save Master Volume.")
                  : qsTr("Master volume is part of each preset and changes when you load one.")
        SettingsChoiceRow {
            title: qsTr("Storage")
            options: [{ text: qsTr("Independent"), value: 0 }, { text: qsTr("With Preset"), value: 1 }]
            value: ctx ? ctx.masterVolumeMode : 0
            onChosen: ctx.edit("masterVolumeMode", value)
        }
    }

    SettingsSection {
        title: qsTr("Hardware Configuration")
        enabled: bridge.connected
        footnote: ctx && ctx.outputConfigMode === 0
                  ? qsTr("Pins, output types, clocks, input configuration and output limiters are stored once for the board. Choose this when your hardware is fixed.")
                  : qsTr("The hardware configuration is part of each preset and changes when you load one.")
        SettingsChoiceRow {
            title: qsTr("Storage")
            options: [{ text: qsTr("Independent"), value: 0 }, { text: qsTr("With Preset"), value: 1 }]
            value: ctx ? ctx.outputConfigMode : 1
            onChosen: ctx.edit("outputConfigMode", value)
        }
    }

    // -- WP QD5 --
    onVisibleChanged: if (visible) outputLatency.refresh()
    Component.onCompleted: outputLatency.refresh()
    SettingsSection {
        visible: outputLatency.supported && outputLatency.choices.length > 0
        title: qsTr("Latency")
        enabled: bridge.connected && !outputLatency.applying
        footnote: outputLatency.caption
        SettingsChoiceRow {
            objectName: "latencyRow"
            title: qsTr("Target")
            detail: outputLatency.applying ? qsTr("Applying…") : ""
            options: outputLatency.choices
            value: outputLatency.shownId
            onChosen: { ctx.saveError = ""; outputLatency.edit(value) }
        }
    }

    SettingsSection {
        visible: page.hw.dacMuteSupported === true
        title: qsTr("External Mute Control")
        enabled: bridge.connected
        footnote: qsTr("Stored on the board, not in presets, and kept through a factory reset.")

        SettingsRow {
            title: qsTr("Adjust only with audio stopped")
            titleColor: isMacOS ? MacColors.label : "#ff9f0a"
            detail: qsTr("Changing these settings while audio is playing can send a loud pop or full-level transient to your amplifier and speakers. Stop playback before making changes.")
            Icon { name: "warning"; size: 18; color: "#ff9f0a" }
        }
        SettingsSwitchRow {
            title: qsTr("Enable Automatic Mute")
            detail: qsTr("Briefly mute an external DAC or amplifier to suppress loud pops during system state changes.")
            checked: ctx ? ctx.dacEnabled : false
            onToggled: {
                ctx.edit("dacEnabled", checked)
                // No pin yet: take the first free one
                if (checked && ctx.dacPin === 0xFF) {
                    var o = ctx.pinOptions(0xFF)
                    if (o.length > 0) ctx.edit("dacPin", o[0].value)
                }
            }
        }
        SettingsChoiceRow {
            visible: ctx ? ctx.dacEnabled : false
            title: qsTr("Polarity")
            detail: qsTr("Active Low for PCM5102A or WM8741; Active High for the AK4493 default.")
            options: [{ text: qsTr("Active Low"), value: true }, { text: qsTr("Active High"), value: false }]
            value: ctx ? ctx.dacActiveLow : true
            onChosen: ctx.edit("dacActiveLow", value)
        }
        SettingsPinRow {
            visible: ctx ? ctx.dacEnabled : false
            title: qsTr("Mute Pin")
            detail: qsTr("GPIO wired to the DAC or amplifier's mute input.")
            pin: ctx ? ctx.dacPin : 0xFF
            sharable: ["DAC Mute"]
            onChosen: ctx.edit("dacPin", pin)
        }
        SettingsChoiceRow {
            visible: ctx ? ctx.dacEnabled : false
            title: qsTr("Hold Time")
            detail: qsTr("Wait after muting before the clocks stop.")
            options: page.msOptions([5, 10, 20, 50, 100], ctx ? ctx.dacHoldMs : 5)
            value: ctx ? ctx.dacHoldMs : 5
            onChosen: ctx.edit("dacHoldMs", value)
        }
        SettingsChoiceRow {
            visible: ctx ? ctx.dacEnabled : false
            title: qsTr("Release Time")
            detail: qsTr("Wait after un-muting before audio resumes.")
            options: page.msOptions([0, 5, 10, 20, 50, 100], ctx ? ctx.dacReleaseMs : 0)
            value: ctx ? ctx.dacReleaseMs : 0
            onChosen: ctx.edit("dacReleaseMs", value)
        }
        SettingsButtonRow {
            visible: ctx ? ctx.dacEnabled : false
            title: qsTr("Test")
            detail: ctx && ctx.dacDirty ? qsTr("Save your changes first to test the mute pin.")
                                        : qsTr("Toggle the mute output for one second to confirm the wiring.")
            enabled: page.hw.dacMuteEnabled === true && !(ctx && ctx.dacDirty)
            buttonText: qsTr("Start")
            onClicked: {
                var code = bridge.testDacMute()
                page.dacStatusError = code !== 0
                page.dacStatus = code === 0 ? qsTr("Mute output pulsed for one second.")
                               : code === 3 ? qsTr("Enable and save the mute first.") : ctx.statusText(code)
            }
        }
    }

    SettingsStatusLine { text: page.dacStatus; error: page.dacStatusError }
}
