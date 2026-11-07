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
    title: "Overview"
    subtitle: "Every GPIO the device has claimed, by what uses it."

    readonly property var roles: [
        { key: "output", title: "Outputs", color: "#0278c7" },
        { key: "clock", title: "Clocks", color: "#ba3822" },
        { key: "input", title: "Inputs", color: "#04856f" },
        { key: "control", title: "Control", color: "#9543a7" },
        { key: "other", title: "Other", color: "#807701" }]
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
        if (code < 0x06 || code > 0x0F) return "code 0x" + ("0" + code.toString(16).toUpperCase()).slice(-2)
        return ((850 + 50 * (code - 0x06)) / 1000).toFixed(2) + " V"
    }
    function liveVolts() {
        return vreg.millivolts > 0 ? (vreg.millivolts / 1000).toFixed(2) + " V" : vregVolts(vreg.liveVreg)
    }
    function formatUptime(s) {
        var d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60), sec = s % 60
        if (d > 0) return d + " d " + h + " h"
        if (h > 0) return h + " h " + m + " min"
        if (m > 0) return m + " min " + sec + " s"
        return sec + " s"
    }
    SettingsSection {
        objectName: "coreVoltageSection"
        visible: ext.hasCoreVreg
        title: "Core Voltage"
        footnote: "At its 307.2 MHz clock the device raises its stored voltage one step by itself after two watchdog hangs in one power session, up to 1.25 V."

        SettingsValueRow {
            objectName: "coreVregRunning"
            title: "Running at"
            value: page.vregRead ? page.liveVolts() + (page.vreg.trialActive ? " (trial)" : "")
                                 : ext.coreVregFailed ? "The device did not answer" : "Reading…"
        }
        SettingsValueRow {
            objectName: "coreVregTrial"
            visible: page.vregRead && page.vreg.trialActive && page.vreg.trialVreg !== page.vreg.liveVreg
            title: "Trial voltage"
            value: visible ? page.vregVolts(page.vreg.trialVreg) : ""
        }
        SettingsValueRow {
            objectName: "coreVregStored"
            visible: page.vregRead
            title: "Stored"
            value: !page.vregRead ? "" : page.vreg.storedSel === 0 ? "Build default"
                 : page.vregVolts(page.vreg.storedSel) + (page.vreg.autoRaised ? ", raised by the device" : "")
        }
        SettingsValueRow {
            objectName: "coreVregDefault"
            visible: page.vregRead
            title: "Build default"
            value: page.vregRead ? page.vregVolts(page.vreg.defaultVreg) : ""
        }
        SettingsValueRow {
            objectName: "coreVregAuto"
            visible: page.vregRead
            title: "Automatic raising"
            value: page.vreg.autoDisabled ? "Off" : "On"
        }
        SettingsValueRow {
            objectName: "coreVregHangs"
            visible: page.vregRead
            title: "Hangs this session"
            value: page.vregRead ? String(page.vreg.hangCount) : ""
        }
        SettingsValueRow {
            objectName: "coreVregFailedTrial"
            visible: page.vregRead
            title: "Last failed trial"
            value: !page.vregRead || page.vreg.failedTrialVreg === 0 ? "None"
                 : page.vregVolts(page.vreg.failedTrialVreg)
                   + (page.vreg.failedTrialUptimeS > 0 ? ", hung after " + page.formatUptime(page.vreg.failedTrialUptimeS) : "")
        }
        SettingsValueRow {
            objectName: "coreVregUptime"
            visible: page.vregRead
            title: "Uptime"
            value: page.vregRead ? page.formatUptime(page.vreg.uptimeS) : ""
        }
    }

    // ── Map ──
    SettingsSection {
        title: bridge.connected ? page.used + " of " + bridge.validPins.length + " GPIOs in use" : "Pins"
        footnote: bridge.connected ? (bridge.validPins.length - page.used) + " free. GPIO 23-25 are wired inside the Pico and not offered."
                                   : "Pin assignments live on the device. Connect a DSPi to see which GPIOs are in use."

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
                            text: "GP" + modelData
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
                        ToolTip.text: "GPIO " + modelData + ": " + (owner ? owner.owner : "free")
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
                    value: "GPIO " + modelData.pin
                }
            }
        }
    }

    Text {
        visible: bridge.connected && page.owners.length === 0
        text: "No GPIOs are currently claimed."
        font.pixelSize: 13
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
}
