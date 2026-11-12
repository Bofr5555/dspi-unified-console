import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Advanced (the Mac's AdvancedSettingsTab): reset the channel
// names, the debug switch, and on the desktop the open device's facts and the
// Core Voltage entry (behaviour.md §8). Sections other packages add (the
// onboarding developer rows, behind the debug switch) go below.
SettingsPage {
    id: page
    objectName: "AdvancedPage"
    readonly property var vreg: SettingsController.coreVoltage

    SettingsGroup {
        title: qsTr("Channel Names")
        iconName: "list-ordered"
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("Reset all channel names to factory defaults.")
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "Advanced.resetNames"
                text: qsTr("Reset")
                enabled: DeviceModel.connected && !DeviceModel.readOnly
                onClicked: SettingsController.resetChannelNames()
            }
        }
    }

    SettingsGroup {
        title: qsTr("Diagnostics")
        iconName: "activity"
        SettingsToggle {
            objectName: "Advanced.debug"
            title: qsTr("Show Debug Information")
            caption: qsTr("Display additional diagnostic data in the UI")
            checked: SettingsController.showDebugInfo
            onToggled: on => SettingsController.showDebugInfo = on
        }
    }

    SettingsGroup {
        objectName: "Advanced.device"
        visible: DeviceModel.connected
        title: qsTr("Device")
        iconName: "hard-drive"
        SettingsValueRow {
            title: qsTr("Platform")
            value: DeviceModel.platformName
        }
        SettingsValueRow {
            objectName: "Advanced.firmware"
            title: qsTr("Firmware")
            value: DeviceModel.firmwareVersion
        }
        SettingsValueRow {
            title: qsTr("Serial Number")
            value: DeviceModel.serial
            mono: true
        }
        SettingsValueRow {
            title: qsTr("Channels")
            value: qsTr("%1 in, %2 out").arg(DeviceModel.numInputs).arg(DeviceModel.numOutputs)
        }
    }

    // The entry to the Core Voltage page, once a package registers it, on
    // firmware with the extension (behaviour.md §20)
    SettingsGroup {
        objectName: "Advanced.coreVoltage"
        visible: DeviceModel.connected && page.vreg.supported === true
                 && SettingsController.pageType("coreVoltage").length > 0
        title: qsTr("Core Voltage")
        iconName: "gauge"
        RowLayout {
            Layout.fillWidth: true
            Text {
                Layout.fillWidth: true
                text: page.vreg.read === true
                      ? qsTr("Running at %1").arg(SettingsController.voltsLabel(page.vreg.liveVreg))
                        + (page.vreg.trialActive ? " " + qsTr("(trial)") : "")
                      : qsTr("Reading…")
                font: Theme.fontBody
                color: Theme.text
            }
            AppButton {
                objectName: "Advanced.coreVoltageOpen"
                text: qsTr("Open…")
                onClicked: SettingsController.open("coreVoltage")
            }
        }
    }

    SettingsExtensions {
        page: "advanced"
    }
}
