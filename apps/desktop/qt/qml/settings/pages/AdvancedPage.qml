import QtQuick 2.15
import "../"

// -- WP QD28 -- An Item round the page, so Core Voltage can open in its place: that
// page is reachable from here only, with no entry of its own in the window.
Item {
    id: advanced
    property var ctx                    // SettingsContext, set by the window
    readonly property string title: "Advanced"
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
            title: "Device"
            enabled: bridge.connected
            SettingsValueRow { title: "Platform"; value: bridge.connected ? bridge.platformName : "—" }
            SettingsValueRow { title: "Firmware"; value: bridge.firmwareVersion || "—" }
            SettingsValueRow { title: "Serial Number"; value: bridge.selectedSerial || "—"; mono: true }
            SettingsValueRow {
                title: "Channels"
                value: bridge.connected ? bridge.numInputChannels + " inputs, " + bridge.numOutputChannels + " outputs" : "—"
            }
        }

        SettingsSection {
            title: "Channel Names"
            footnote: "Renames every channel back to its default (USB L, SPDIF 1 L and so on). Takes effect immediately."
            SettingsButtonRow {
                title: "Reset Channel Names"
                buttonText: "Reset"
                enabled: bridge.connected
                onClicked: bridge.resetChannelNames()
            }
        }

        // -- WP QD28 --
        SettingsSection {
            objectName: "advancedCoreVoltage"
            visible: advanced.coreVoltageOffered || vreg.phase !== "idle"
            title: "Core Voltage"
            footnote: "Changes the voltage of the device's processor. For a device that hangs or drops off USB; read the warning first."
            SettingsButtonRow {
                objectName: "advancedCoreVoltageOpen"
                title: "Core Voltage"
                detail: {
                    var s = vreg.view.status
                    if (vreg.phase !== "idle" && vreg.phase !== "review") return "Calibration: " + (vreg.view.message || vreg.phase)
                    if (s && s.trialActive) return "A trial is running at " + vreg.volts(s.trialVreg)
                    return s ? "Running at " + (s.millivolts > 0 ? (s.millivolts / 1000).toFixed(2) + " V" : vreg.volts(s.liveVreg)) : ""
                }
                buttonText: "Open…"
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
