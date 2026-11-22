import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5a -- What stands over the Overview's channel cards (MODLAY 2): the
// device, its effects as chips that open their pages, and how it is connected
// (the hub and its audio link, when it is a hub's). `host` is the page host.
GridLayout {
    id: summary
    objectName: "OverviewSummary"
    property Item host: null

    columns: width >= 760 ? 3 : 1
    columnSpacing: DashboardController.spacing
    rowSpacing: Theme.spacingSmall

    function healthColor(h) {
        return h === "ok" ? Theme.success : h === "warning" ? Theme.warning : h === "problem" ? Theme.dangerText : Theme.offline
    }

    component Box: Rectangle {
        id: box
        property string title
        default property alias content: rows.data
        Layout.fillWidth: true
        Layout.fillHeight: true
        // Thirds, whatever is in them
        Layout.preferredWidth: 1
        implicitHeight: rows.implicitHeight + 2 * Theme.spacing
        radius: 10
        color: Theme.panel
        ColumnLayout {
            id: rows
            anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.spacing }
            spacing: 5
            Text {
                Layout.bottomMargin: 3
                text: box.title
                font: Theme.fontHeading
                color: Theme.textMuted
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
        }
        Accessible.role: Accessible.Grouping
        Accessible.name: title
    }
    // A fact: what, and its value
    component Fact: RowLayout {
        id: fact
        property string label
        property string value
        property color ink: Theme.text
        // For the smoke driver
        readonly property string shownName: value
        Layout.fillWidth: true
        visible: value.length > 0
        spacing: Theme.spacingSmall
        Text {
            text: fact.label
            font: Theme.fontBody
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignRight
            textFormat: Text.PlainText
            text: fact.value
            font: Theme.fontBody
            color: fact.ink
            elide: Text.ElideRight
        }
        Accessible.role: Accessible.StaticText
        Accessible.name: label + ": " + value
    }

    Box {
        objectName: "OverviewSummary.device"
        title: qsTr("DEVICE")
        Fact {
            objectName: "OverviewSummary.name"
            label: qsTr("Name")
            value: DeviceModel.deviceTitle
        }
        Fact {
            objectName: "OverviewSummary.board"
            label: qsTr("Board")
            value: [DeviceInfoController.boardName, DeviceModel.platformName].filter((v, i, all) => v.length && all.indexOf(v) === i).join(" · ")
        }
        Fact {
            objectName: "OverviewSummary.firmware"
            label: qsTr("Firmware")
            value: DeviceModel.firmwareVersion
        }
        Fact {
            objectName: "OverviewSummary.preset"
            readonly property int slot: DeviceModel.presets.activeSlot
            readonly property string title: slot >= 0 && slot < SidebarController.presetTitles.length ? SidebarController.presetTitles[slot] : ""
            label: qsTr("Preset")
            value: title   // with a * while there are unsaved changes
        }
    }

    Box {
        objectName: "OverviewSummary.effects"
        title: qsTr("EFFECTS")
        Flow {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: EffectsState.available
                AbstractButton {
                    id: chip
                    required property var modelData
                    readonly property bool on: EffectsState.on(modelData.tab) === true
                    objectName: "OverviewSummary.effect." + modelData.tab
                    implicitHeight: 26
                    leftPadding: 10
                    rightPadding: 10
                    text: modelData.title
                    hoverEnabled: true
                    focusPolicy: Qt.TabFocus
                    onClicked: if (summary.host) summary.host.go("effects", modelData.tab)
                    contentItem: Row {
                        spacing: 6
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 7
                            height: 7
                            radius: 3.5
                            color: chip.on ? Theme.success : "transparent"
                            border.width: chip.on ? 0 : 1
                            border.color: Theme.textMuted
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            textFormat: Text.PlainText
                            text: chip.text
                            font: Theme.fontBody
                            color: chip.on ? Theme.text : Theme.textMuted
                        }
                    }
                    background: Rectangle {
                        radius: height / 2
                        color: chip.hovered ? Theme.controlHover : Theme.control
                        border.width: chip.visualFocus ? 2 : 0
                        border.color: Theme.focusRing
                    }
                    Accessible.role: Accessible.Button
                    Accessible.name: on ? qsTr("%1, on").arg(text) : qsTr("%1, off").arg(text)
                    Accessible.onPressAction: chip.clicked()
                }
            }
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("Choose one to open its page.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    Box {
        objectName: "OverviewSummary.connection"
        title: qsTr("CONNECTION")
        readonly property bool hub: DeviceModel.remote
        id: connection
        Fact {
            objectName: "OverviewSummary.transport"
            label: qsTr("Through")
            value: FilesController.offline ? qsTr("File") : DeviceModel.demo ? qsTr("Demo") : connection.hub ? qsTr("DSPi Link") : qsTr("USB")
        }
        Fact {
            objectName: "OverviewSummary.hub"
            label: qsTr("Hub")
            value: connection.hub ? LinkController.hubName : ""
        }
        Fact {
            objectName: "OverviewSummary.security"
            label: qsTr("Security")
            value: !connection.hub ? "" : LinkController.encrypted ? qsTr("Encrypted")
                   : LinkController.unsecured ? LinkController.unsecuredLabel : LinkController.local ? qsTr("This computer") : ""
            ink: connection.hub && LinkController.unsecured ? Theme.dangerText : Theme.text
        }
        // The hub's audio link in one line of the core's ("Audio link: ..."); its page
        AbstractButton {
            id: alink
            objectName: "OverviewSummary.alink"
            property string shownName: text
            Layout.fillWidth: true
            visible: connection.hub && SystemController.available && SystemController.installed
            text: SystemController.summary
            hoverEnabled: true
            focusPolicy: Qt.TabFocus
            onClicked: if (summary.host) summary.host.go("system", "status")
            contentItem: RowLayout {
                spacing: Theme.spacingSmall
                Rectangle {
                    Layout.alignment: Qt.AlignTop
                    Layout.topMargin: 5
                    implicitWidth: 8
                    implicitHeight: 8
                    radius: 4
                    color: summary.healthColor(SystemController.health)
                }
                Text {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: alink.text
                    font: Theme.fontBody
                    color: alink.hovered ? Theme.accentText : Theme.text
                    wrapMode: Text.Wrap
                }
            }
            background: Rectangle {
                radius: Theme.radiusSmall
                color: "transparent"
                border.width: alink.visualFocus ? 2 : 0
                border.color: Theme.focusRing
            }
            Accessible.role: Accessible.Button
            Accessible.name: text
            Accessible.onPressAction: alink.clicked()
        }
        Fact {
            objectName: "OverviewSummary.alarms"
            label: qsTr("Alarms")
            value: connection.hub && SystemController.alarms.length > 0 ? String(SystemController.alarms.length) : ""
            ink: Theme.alarmRaised
        }
        AppButton {
            objectName: "OverviewSummary.devices"
            Layout.topMargin: 2
            text: qsTr("Manage Devices…")
            onClicked: if (summary.host) summary.host.go("devices", "devices")
        }
    }
}
