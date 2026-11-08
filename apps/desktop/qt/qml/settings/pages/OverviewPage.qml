import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import "../"
import "../../components"

// Which GPIO does what: a map of the wirable pins coloured by role, then a
// list per role. Read-only; change pins on the Inputs, Outputs, I2S and
// Control Interfaces pages.
SettingsPage {
    writesToDevice: false   // -- WP QD6 --
    id: page
    title: qsTr("Overview")
    subtitle: qsTr("Every GPIO the device has claimed, by what uses it.")

    readonly property var roles: [
        { key: "output", title: qsTr("Outputs"), color: "#0278c7" },
        { key: "clock", title: qsTr("Clocks"), color: "#ba3822" },
        { key: "input", title: qsTr("Inputs"), color: "#04856f" },
        { key: "control", title: qsTr("Control"), color: "#9543a7" },
        { key: "other", title: qsTr("Other"), color: "#807701" }]
    function roleColor(key) {
        for (var i = 0; i < roles.length; i++) if (roles[i].key === key) return roles[i].color
        return "#807701"
    }

    // Not bound to bridge.connected directly: that notifies on every status poll
    readonly property bool connected: bridge.connected
    readonly property var owners: { bridge.hardware; controlSurfaces.revision; return connected ? bridge.pinOwners() : [] }
    readonly property var byPin: {
        var m = {}
        for (var i = 0; i < owners.length; i++) if (m[owners[i].pin] === undefined) m[owners[i].pin] = owners[i]
        return m
    }
    readonly property int used: {
        var n = 0, pins = bridge.validPins
        for (var i = 0; i < pins.length; i++) if (byPin[pins[i]] !== undefined) n++
        return n
    }

    // -- WP QD3 -- ── Core voltage ──
    // The status only, first on the page as on the Mac; writes are QD28's, in Advanced.
    // Read while this page shows in a window that's open.
    readonly property bool coreVregShown: Window.visibility !== Window.Hidden && Window.visibility !== Window.Minimized
    onCoreVregShownChanged: ext.coreVregWatching = coreVregShown
    Component.onCompleted: ext.coreVregWatching = coreVregShown
    Component.onDestruction: ext.coreVregWatching = false
    readonly property var vreg: ext.coreVreg
    readonly property bool vregRead: vreg.liveVreg !== undefined
    // Volts for a VREG code, as dspi_proto::ext::vreg_mv; the C API has it only for
    // the live code, as `millivolts`
    function vregVolts(code) {
        if (code < 0x06 || code > 0x0F) return qsTr("code 0x%1").arg(("0" + code.toString(16).toUpperCase()).slice(-2))
        return qsTr("%1 V").arg(((850 + 50 * (code - 0x06)) / 1000).toFixed(2))
    }
    function liveVolts() {
        return vreg.millivolts > 0 ? qsTr("%1 V").arg((vreg.millivolts / 1000).toFixed(2)) : vregVolts(vreg.liveVreg)
    }
    function formatUptime(s) {
        var d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60), sec = s % 60
        if (d > 0) return qsTr("%1 d %2 h").arg(d).arg(h)
        if (h > 0) return qsTr("%1 h %2 min").arg(h).arg(m)
        if (m > 0) return qsTr("%1 min %2 s").arg(m).arg(String(sec))
        return qsTr("%1 s").arg(String(sec))
    }
    SettingsSection {
        objectName: "coreVoltageSection"
        visible: ext.hasCoreVreg
        title: qsTr("Core Voltage")
        footnote: qsTr("At its 307.2 MHz clock the device raises its stored voltage one step by itself after two watchdog hangs in one power session, up to 1.25 V.")

        SettingsValueRow {
            objectName: "coreVregRunning"
            title: qsTr("Running at")
            value: page.vregRead ? (page.vreg.trialActive ? qsTr("%1 (trial)").arg(page.liveVolts()) : page.liveVolts())
                                 : ext.coreVregFailed ? qsTr("The device did not answer") : qsTr("Reading…")
        }
        SettingsValueRow {
            objectName: "coreVregTrial"
            visible: page.vregRead && page.vreg.trialActive && page.vreg.trialVreg !== page.vreg.liveVreg
            title: qsTr("Trial voltage")
            value: visible ? page.vregVolts(page.vreg.trialVreg) : ""
        }
        SettingsValueRow {
            objectName: "coreVregStored"
            visible: page.vregRead
            title: qsTr("Stored")
            value: !page.vregRead ? "" : page.vreg.storedSel === 0 ? qsTr("Build default")
                 : (page.vreg.autoRaised ? qsTr("%1, raised by the device").arg(page.vregVolts(page.vreg.storedSel)) : page.vregVolts(page.vreg.storedSel))
        }
        SettingsValueRow {
            objectName: "coreVregDefault"
            visible: page.vregRead
            title: qsTr("Build default")
            value: page.vregRead ? page.vregVolts(page.vreg.defaultVreg) : ""
        }
        SettingsValueRow {
            objectName: "coreVregAuto"
            visible: page.vregRead
            title: qsTr("Automatic raising")
            value: page.vreg.autoDisabled ? qsTr("Off") : qsTr("On")
        }
        SettingsValueRow {
            objectName: "coreVregHangs"
            visible: page.vregRead
            title: qsTr("Hangs this session")
            value: page.vregRead ? String(page.vreg.hangCount) : ""
        }
        SettingsValueRow {
            objectName: "coreVregFailedTrial"
            visible: page.vregRead
            title: qsTr("Last failed trial")
            value: !page.vregRead || page.vreg.failedTrialVreg === 0 ? qsTr("None")
                 : (page.vreg.failedTrialUptimeS > 0 ? qsTr("%1, hung after %2").arg(page.vregVolts(page.vreg.failedTrialVreg)).arg(page.formatUptime(page.vreg.failedTrialUptimeS))
                   : page.vregVolts(page.vreg.failedTrialVreg))
        }
        SettingsValueRow {
            objectName: "coreVregUptime"
            visible: page.vregRead
            title: qsTr("Uptime")
            value: page.vregRead ? page.formatUptime(page.vreg.uptimeS) : ""
        }
    }

    // ── Map ──
    SettingsSection {
        title: bridge.connected ? qsTr("%1 of %2 GPIOs in use").arg(page.used).arg(bridge.validPins.length) : qsTr("Pins")
        footnote: bridge.connected ? qsTr("%1 free. GPIO 23-25 are wired inside the Pico and not offered.").arg(bridge.validPins.length - page.used)
                                   : qsTr("Pin assignments live on the device. Connect a DSPi to see which GPIOs are in use.")

        Item {
            width: parent.width
            height: map.height + 28
            Flow {
                id: map
                x: 14
                y: 14
                width: parent.width - 28
                spacing: 6
                Repeater {
                    model: bridge.validPins
                    Rectangle {
                        readonly property var owner: page.byPin[modelData]
                        width: 52
                        height: 30
                        radius: 7
                        color: isMacOS ? (owner ? page.roleColor(owner.role) : MacColors.opacity(MacColors.secondaryLabel, 0.12)) : owner ? Qt.darker(page.roleColor(owner.role), 1.15) : Qt.rgba(1, 1, 1, 0.05)
                        border.color: isMacOS ? "transparent" : owner ? Qt.lighter(page.roleColor(owner.role), 1.3) : Qt.rgba(1, 1, 1, 0.08)
                        Text {
                            anchors.centerIn: parent
                            text: qsTr("GP%1").arg(modelData)
                            font.pixelSize: 11
                            font.weight: Font.DemiBold
                            color: isMacOS ? (parent.owner ? "white" : MacColors.secondaryLabel) : parent.owner ? "white" : Qt.rgba(1, 1, 1, 0.4)
                        }
                        MouseArea {
                            id: chipHover
                            anchors.fill: parent
                            hoverEnabled: true
                        }
                        ToolTip.visible: chipHover.containsMouse
                        ToolTip.delay: 300
                        ToolTip.text: owner ? qsTr("GPIO %1: %2").arg(modelData).arg(owner.owner) : qsTr("GPIO %1: free").arg(modelData)
                    }
                }
            }
        }
        // Legend
        Item {
            width: parent.width
            height: 34
            Row {
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 14
                Repeater {
                    model: page.roles
                    Row {
                        spacing: 5
                        Rectangle { width: 10; height: 10; radius: 3; color: modelData.color; anchors.verticalCenter: parent.verticalCenter }
                        Text { text: modelData.title; font.pixelSize: 11; color: Qt.rgba(1, 1, 1, 0.6) }
                    }
                }
            }
        }
    }

    // ── Per role ──
    Repeater {
        model: bridge.connected ? page.roles : []
        SettingsSection {
            id: roleSection
            readonly property var members: page.owners.filter(function(o) { return o.role === modelData.key })
                                                      .sort(function(a, b) { return a.pin - b.pin })
            visible: members.length > 0
            title: modelData.title
            Repeater {
                model: roleSection.members
                SettingsValueRow {
                    title: modelData.owner
                    value: qsTr("GPIO %1").arg(modelData.pin)
                }
            }
        }
    }

    Text {
        visible: bridge.connected && page.owners.length === 0
        text: qsTr("No GPIOs are currently claimed.")
        font.pixelSize: 13
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
}
