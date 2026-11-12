import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- Settings > Global Parameters (the Mac's GlobalSettingsTab):
// the device's name, the output latency, and under one "At power-on" heading
// what the device does when it starts: the startup preset, how the master
// volume and the hardware configuration are kept, and the external mute.
// Every change is held for the save bar.
SettingsPage {
    id: page
    objectName: "GlobalPage"
    readonly property var hw: HardwareController
    readonly property var draft: hw.draft
    readonly property bool editable: hw.writable

    Component.onCompleted: {
        hw.watch(true)
        OutputLatencyController.refresh()
    }
    Component.onDestruction: hw.watch(false)

    // ---- Device Name
    SettingsGroup {
        objectName: "Global.deviceName"
        visible: NamesController.deviceNameSupported
        title: qsTr("Device Name")
        iconName: "cpu"
        footer: qsTr("Stored on the device, so it keeps this name with any console or DSPi Link hub. Shown in the device picker and on the front panel.")
        HwRow {
            title: qsTr("Name")
            caption: DeviceModel.serial.length ? qsTr("Serial %1").arg(DeviceModel.serial) : ""
            enabled: page.editable
            NameField {
                objectName: "Global.name"
                label: qsTr("Device Name")
                text: NamesController.deviceName
                edited: NamesController.deviceNameEdited
                clearHelp: qsTr("Remove the name")
                edit: t => NamesController.setDeviceName(t)
            }
        }
    }

    // ---- Latency (behaviour.md §16)
    SettingsGroup {
        objectName: "Global.latency"
        visible: OutputLatencyController.supported
        title: qsTr("Latency")
        iconName: "timer"
        HwRow {
            title: qsTr("Target")
            enabled: page.editable
            ChoicePicker {
                objectName: "Global.latencyTarget"
                label: qsTr("Latency target")
                implicitWidth: 150
                choices: OutputLatencyController.options.map(o => ({ value: o.id, text: o.text }))
                value: OutputLatencyController.selectedId
                missingText: "-"
                onChosen: v => OutputLatencyController.choose(v)
            }
        }
        Text {
            objectName: "Global.latencyCaption"
            Layout.fillWidth: true
            text: OutputLatencyController.caption
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    // ---- At power-on (PRO1 S5: everything the device does when it starts)
    SectionHeading {
        objectName: "Global.powerOn"
        Layout.fillWidth: true
        Layout.topMargin: Theme.spacingSmall
        text: qsTr("At power-on")
        rule: true
    }

    SettingsGroup {
        objectName: "Global.startup"
        title: qsTr("Startup Preset")
        iconName: "power"
        enabled: page.editable
        HwRow {
            title: qsTr("Mode")
            ChoicePicker {
                objectName: "Global.startupMode"
                label: qsTr("Startup preset mode")
                implicitWidth: 170
                choices: [{ value: 0, text: qsTr("Specified Default") }, { value: 1, text: qsTr("Last Used") }]
                value: page.draft.startupMode
                onChosen: v => page.hw.setDraft("startupMode", v)
            }
        }
        HwRow {
            visible: page.draft.startupMode === 0
            title: qsTr("Default Preset")
            ChoicePicker {
                objectName: "Global.defaultSlot"
                label: qsTr("Default Preset")
                implicitWidth: 170
                choices: page.hw.presetSlotLabels.map((t, i) => ({ value: i, text: t }))
                value: page.draft.defaultSlot
                onChosen: v => page.hw.setDraft("defaultSlot", v)
            }
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("Choose which preset loads when the device powers on.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    SettingsGroup {
        objectName: "Global.masterVolume"
        title: qsTr("Master Volume")
        iconName: "volume-2"
        enabled: page.editable
        HwRow {
            title: qsTr("Mode")
            ChoicePicker {
                objectName: "Global.masterVolumeMode"
                label: qsTr("Master volume mode")
                implicitWidth: 170
                choices: [{ value: 0, text: qsTr("Independent") }, { value: 1, text: qsTr("With Preset") }]
                value: page.draft.masterVolumeMode
                onChosen: v => page.hw.setDraft("masterVolumeMode", v)
            }
        }
        Text {
            Layout.fillWidth: true
            text: page.draft.masterVolumeMode === 0
                  ? qsTr("Master volume is stored on the device independently of presets and applied at boot. Loading a preset never changes it.")
                  : qsTr("Master volume is part of each preset. Saved with the preset, restored on preset load.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    SettingsGroup {
        objectName: "Global.outputConfig"
        title: qsTr("Hardware Configuration")
        iconName: "cable"
        enabled: page.editable
        HwRow {
            title: qsTr("Mode")
            ChoicePicker {
                objectName: "Global.outputConfigMode"
                label: qsTr("Hardware configuration mode")
                implicitWidth: 170
                choices: [{ value: 0, text: qsTr("Independent") }, { value: 1, text: qsTr("With Preset") }]
                value: page.draft.outputConfigMode
                onChosen: v => page.hw.setDraft("outputConfigMode", v)
            }
        }
        Text {
            Layout.fillWidth: true
            text: page.draft.outputConfigMode === 0
                  ? qsTr("Input and output configuration is stored on the device independently and applied at boot. Loading a preset never changes your wiring.")
                  : qsTr("Input and output configuration is part of each preset. Saved with the preset and restored on preset load.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    // ---- External Mute Control (firmware with the DAC hardware mute)
    SettingsGroup {
        objectName: "Global.mute"
        visible: page.hw.has.dacMute === true
        title: qsTr("External Mute Control")
        iconName: "volume-x"
        enabled: page.editable

        // Moving the pin or the polarity while the DAC runs can send a loud
        // transient straight into the amplifier
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Icon {
                Layout.alignment: Qt.AlignTop
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Adjust only with audio stopped")
                    font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Changing these settings while audio is playing can send a loud pop or full-level transient to your amplifier and speakers. Stop playback before making changes.")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
        SettingsToggle {
            objectName: "Global.muteEnabled"
            title: qsTr("Enable Automatic Mute")
            caption: qsTr("Briefly mute an external DAC or amplifier to suppress loud pops during system state changes.")
            checked: page.draft.muteEnabled
            onToggled: on => page.hw.setDraft("muteEnabled", on)
        }
        HwRow {
            visible: page.draft.muteEnabled
            title: qsTr("Polarity")
            ChoicePicker {
                objectName: "Global.mutePolarity"
                label: qsTr("Polarity")
                choices: [{ value: true, text: qsTr("Active Low") }, { value: false, text: qsTr("Active High") }]
                value: page.draft.muteActiveLow
                onChosen: v => page.hw.setDraft("muteActiveLow", v)
            }
        }
        HwRow {
            visible: page.draft.muteEnabled
            title: qsTr("Mute Pin")
            PinPicker {
                objectName: "Global.mutePin"
                label: qsTr("Mute Pin")
                pins: { page.hw.revision; return page.hw.pinChoices(HardwareController.DacMute, 0, page.draft.mutePin) }
                pin: page.draft.mutePin
                onPicked: p => page.hw.setDraft("mutePin", p)
            }
        }
        HwRow {
            visible: page.draft.muteEnabled
            title: qsTr("Hold Time")
            caption: qsTr("Mute-attack wait before clock-stop")
            ChoicePicker {
                objectName: "Global.muteHold"
                label: qsTr("Hold Time")
                implicitWidth: 92
                choices: [5, 10, 20, 50, 100].map(ms => ({ value: ms, text: qsTr("%1 ms").arg(ms) }))
                value: page.draft.muteHoldMs
                missingText: qsTr("%1 ms").arg(page.draft.muteHoldMs)
                onChosen: v => page.hw.setDraft("muteHoldMs", v)
            }
        }
        HwRow {
            visible: page.draft.muteEnabled
            title: qsTr("Release Time")
            caption: qsTr("Wait after un-mute before audio resumes")
            ChoicePicker {
                objectName: "Global.muteRelease"
                label: qsTr("Release Time")
                implicitWidth: 92
                choices: [0, 5, 10, 20, 50, 100].map(ms => ({ value: ms, text: qsTr("%1 ms").arg(ms) }))
                value: page.draft.muteReleaseMs
                missingText: qsTr("%1 ms").arg(page.draft.muteReleaseMs)
                onChosen: v => page.hw.setDraft("muteReleaseMs", v)
            }
        }
        HwRow {
            visible: page.draft.muteEnabled
            title: qsTr("Test")
            caption: qsTr("Toggle automatic mute for one second to confirm hardware configuration.")
            AppButton {
                objectName: "Global.muteTest"
                text: qsTr("Start")
                // The test runs what the device holds: save first
                enabled: !page.draft.dirty && page.draft.mutePin !== 255 && page.draft.muteEnabled
                toolTip: page.draft.dirty ? qsTr("Save your changes first to test the mute pin.")
                                          : qsTr("Begins one-second test.")
                onClicked: page.hw.testDacMute()
            }
        }
    }
}
