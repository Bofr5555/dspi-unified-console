import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Settings > Overview (the Mac's PinOverviewTab and CoreVoltageSection): the
// core voltage status on firmware with the extension, a map of every GPIO in
// pin order with the legend, and the held pins grouped by what holds them.
// Only held pins are listed: a feature that is configured but off holds none.
SettingsPage {
    id: page
    objectName: "OverviewPage"
    readonly property var claims: SettingsController.pinClaims
    readonly property var pins: SettingsController.gpioPins
    readonly property var vreg: SettingsController.coreVoltage
    readonly property var roleTitles: [qsTr("Outputs"), qsTr("Clocks"), qsTr("Inputs"), qsTr("Control"), qsTr("Other")]
    readonly property int freeCount: pins.length - grid.heldCount

    function claimsOf(role) {
        return claims.filter(c => c.role === role)
    }

    Component.onCompleted: SettingsController.refreshDeviceFacts()

    // ---- Core voltage (read only) ----------------------------------------------
    SettingsGroup {
        objectName: "Overview.coreVoltage"
        visible: DeviceModel.connected && page.vreg.supported === true
        title: qsTr("Core Voltage")
        iconName: "gauge"
        footer: qsTr("At its 307.2 MHz clock the device raises its stored voltage one step by itself after two watchdog hangs in one power session, up to 1.25 V.")
        trailing: IconButton {
            objectName: "Overview.coreVoltageRefresh"
            iconName: "refresh-cw"
            iconSize: 13
            toolTip: qsTr("Read the status again")
            onClicked: SettingsController.refreshCoreVoltage()
        }

        Text {
            visible: page.vreg.read !== true
            text: page.vreg.error ? page.vreg.error : qsTr("Reading…")
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        ColumnLayout {
            visible: page.vreg.read === true
            Layout.fillWidth: true
            spacing: 4
            readonly property var v: page.vreg
            function volts(code) { return SettingsController.voltsLabel(code) }
            SettingsValueRow {
                objectName: "Overview.running"
                title: qsTr("Running at")
                value: parent.volts(parent.v.liveVreg) + (parent.v.trialActive ? " " + qsTr("(trial)") : "")
            }
            SettingsValueRow {
                visible: parent.v.trialActive === true && parent.v.trialVreg !== parent.v.liveVreg
                title: qsTr("Trial voltage")
                value: parent.volts(parent.v.trialVreg)
            }
            SettingsValueRow {
                title: qsTr("Stored")
                value: parent.v.storedSel === 0 ? qsTr("Build default")
                       : parent.volts(parent.v.storedSel) + (parent.v.autoRaised ? qsTr(", raised by the device") : "")
            }
            SettingsValueRow {
                title: qsTr("Build default")
                value: parent.volts(parent.v.defaultVreg)
            }
            SettingsValueRow {
                title: qsTr("Automatic raising")
                value: parent.v.autoDisabled ? qsTr("Off") : qsTr("On")
            }
            SettingsValueRow {
                title: qsTr("Hangs this session")
                value: String(parent.v.hangCount)
            }
            SettingsValueRow {
                title: qsTr("Last failed trial")
                value: parent.v.failedTrialVreg
                       ? qsTr("%1, hung after %2").arg(parent.volts(parent.v.failedTrialVreg)).arg(SettingsController.uptimeLabel(parent.v.failedTrialUptimeS))
                       : qsTr("None")
            }
            SettingsValueRow {
                title: qsTr("Uptime")
                value: SettingsController.uptimeLabel(parent.v.uptimeS)
            }
        }
    }

    // ---- Nothing connected -------------------------------------------------------
    SettingsGroup {
        objectName: "Overview.disconnected"
        visible: !DeviceModel.connected
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacing
            Layout.bottomMargin: Theme.spacing
            spacing: 6
            Icon {
                Layout.alignment: Qt.AlignHCenter
                name: "cable"
                size: 28
                color: Theme.textMuted
            }
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("No Device Connected")
                font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                color: Theme.text
            }
            Text {
                Layout.fillWidth: true
                Layout.maximumWidth: 340
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Pin assignments live on the device. Connect a DSPi to see which GPIOs are in use.")
                font: Theme.fontSmall
                color: Theme.textMuted
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
            }
        }
    }

    // ---- The map -------------------------------------------------------------------
    SettingsGroup {
        objectName: "Overview.summary"
        visible: DeviceModel.connected
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Icon {
                name: "cpu"
                size: 15
                color: Theme.accentText
            }
            Text {
                objectName: "Overview.inUse"
                Layout.fillWidth: true
                text: qsTr("%1 of %2 GPIOs in use").arg(grid.heldCount).arg(page.pins.length)
                font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.Medium })
                color: Theme.text
            }
            Text {
                text: page.freeCount === 0 ? qsTr("none free") : qsTr("%1 free").arg(page.freeCount)
                font: Theme.fontSmall
                color: Theme.textMuted
            }
        }
        PinGrid {
            id: grid
            objectName: "Overview.pinGrid"
            Layout.fillWidth: true
            Layout.preferredHeight: implicitHeight
            pins: page.pins
            claims: page.claims
            columns: 9
            chipHeight: 20
            roleColors: Theme.pinRoleTints
            freeColor: Theme.pinFree
            heldTextColor: Theme.textOnAccent
            freeTextColor: Theme.textMuted
            font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontSmall.pixelSize })
            freeText: qsTr("GP%1 - available")

            HelpTip {
                text: grid.hoveredText
                visible: grid.hoveredPin >= 0
            }
            Accessible.role: Accessible.Graphic
            Accessible.name: qsTr("GPIO map")
            Accessible.description: page.claims.map(c => "GP" + c.pin + " " + c.label).join(", ")
        }
        // The legend: one colour per role
        Flow {
            objectName: "Overview.legend"
            Layout.fillWidth: true
            spacing: Theme.spacing
            Repeater {
                model: page.roleTitles
                delegate: Row {
                    required property string modelData
                    required property int index
                    spacing: 4
                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 9
                        height: 9
                        radius: 4.5
                        color: Theme.pinRoleTints[index]
                    }
                    Text {
                        textFormat: Text.PlainText
                        text: modelData
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                }
            }
        }
    }

    Text {
        objectName: "Overview.noneHeld"
        visible: DeviceModel.connected && page.claims.length === 0
        Layout.fillWidth: true
        text: qsTr("No GPIOs are currently claimed.")
        font: Theme.fontSmall
        color: Theme.textMuted
        horizontalAlignment: Text.AlignHCenter
    }

    // ---- The held pins by role ---------------------------------------------------
    Repeater {
        model: DeviceModel.connected ? 5 : 0
        delegate: SettingsGroup {
            id: roleGroup
            required property int index
            readonly property var roleRows: page.claimsOf(index)
            objectName: "Overview.role." + index
            visible: roleRows.length > 0
            title: page.roleTitles[index]
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: Theme.spacingSmall
                rowSpacing: 5
                Repeater {
                    model: roleGroup.roleRows
                    delegate: RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        spacing: 7
                        Rectangle {
                            implicitWidth: 38
                            implicitHeight: 18
                            radius: Theme.radiusSmall
                            color: Theme.pinRoleTints[roleGroup.index]
                            Text {
                                anchors.centerIn: parent
                                text: "GP" + modelData.pin
                                font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontCaption.pixelSize, weight: Font.DemiBold })
                                color: Theme.textOnAccent
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: modelData.label
                            font: Theme.fontSmall
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                    }
                }
            }
        }
    }
}
