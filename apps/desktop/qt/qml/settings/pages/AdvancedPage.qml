import QtQuick 2.15
import "../"

// -- WP QD28 -- An Item round the page, so Core Voltage can open in its place: that
// page is reachable from here only, with no entry of its own in the window.
Item {
    id: advanced
    property var ctx                    // SettingsContext, set by the window
    readonly property string title: qsTr("Advanced")
    property bool coreVoltageOpen: false
    // Only while the core's flow could be used, or a run of it is in a phase
    readonly property bool coreVoltageOffered: bridge.connected && ext.hasCoreVreg && bridge.writable
                                               && (!vreg.polled || vreg.available)

    SettingsPage {
        id: main
        anchors.fill: parent
        visible: !advanced.coreVoltageOpen
        ctx: advanced.ctx
        title: advanced.title

        SettingsSection {
            title: qsTr("Device")
            enabled: bridge.connected
            SettingsValueRow { title: qsTr("Platform"); value: bridge.connected ? bridge.platformName : "—" }
            SettingsValueRow { title: qsTr("Firmware"); value: bridge.firmwareVersion || "—" }
            SettingsValueRow { title: qsTr("Serial Number"); value: bridge.selectedSerial || "—"; mono: true }
            SettingsValueRow {
                title: qsTr("Channels")
                value: bridge.connected ? qsTr("%1 inputs, %2 outputs").arg(bridge.numInputChannels).arg(bridge.numOutputChannels) : "—"
            }
        }

        SettingsSection {
            title: qsTr("Channel Names")
            footnote: qsTr("Renames every channel back to its default (USB L, SPDIF 1 L and so on). Takes effect immediately.")
            SettingsButtonRow {
                title: qsTr("Reset Channel Names")
                buttonText: qsTr("Reset")
                enabled: bridge.connected
                onClicked: bridge.resetChannelNames()
            }
        }

        // -- WP QD28 --
        SettingsSection {
            objectName: "advancedCoreVoltage"
            visible: advanced.coreVoltageOffered || vreg.phase !== "idle"
            title: qsTr("Core Voltage")
            footnote: qsTr("Changes the voltage of the device's processor. For a device that hangs or drops off USB; read the warning first.")
            SettingsButtonRow {
                objectName: "advancedCoreVoltageOpen"
                title: qsTr("Core Voltage")
                detail: {
                    var s = vreg.view.status
                    if (vreg.phase !== "idle" && vreg.phase !== "review") return qsTr("Calibration: %1").arg(vreg.view.message || vreg.phase)
                    if (s && s.trialActive) return qsTr("A trial is running at %1").arg(vreg.volts(s.trialVreg))
                    return s ? qsTr("Running at %1").arg(s.millivolts > 0 ? qsTr("%1 V").arg((s.millivolts / 1000).toFixed(2)) : vreg.volts(s.liveVreg)) : ""
                }
                buttonText: qsTr("Open…")
                onClicked: advanced.coreVoltageOpen = true
            }
        }
    }

    Loader {
        id: coreVoltage
        objectName: "coreVoltageLoader"
        anchors.fill: parent
        active: advanced.coreVoltageOpen
        source: "CoreVoltagePage.qml"
        onLoaded: item.ctx = advanced.ctx
    }
    Connections {
        target: coreVoltage.item
        function onBack() { advanced.coreVoltageOpen = false }
    }
}
