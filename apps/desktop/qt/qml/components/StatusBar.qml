import QtQuick
import QtQuick.Layouts
import UConsole

// The persistent strip along a window's foot (the Pro layer's status bar,
// docs/UI-MODES.md): the connection with its state dot, the scene, the mode,
// then the caller's items at the right.
//
//     StatusBar {
//         connectionText: DeviceModel.deviceTitle
//         connectionState: DeviceModel.connected ? "online" : "offline"
//         modeText: ModeController.modeTitle
//         Text { text: "48 kHz" }
//     }
Rectangle {
    id: bar
    property string connectionText
    // "online", "offline", "stale" or "alarm"
    property string connectionState: "offline"
    property string sceneText
    property string modeText
    default property alias items: right.data

    function stateColor(state) {
        if (state === "online") return Theme.connected
        if (state === "alarm") return Theme.alarmRaised
        if (state === "stale") return Theme.warning
        return Theme.offline
    }
    function stateName(state) {
        if (state === "online") return qsTr("Connected")
        if (state === "alarm") return qsTr("Alarm")
        if (state === "stale") return qsTr("Not updating")
        return qsTr("Not connected")
    }

    implicitHeight: Theme.statusBarHeight
    color: Theme.sidebar
    Rectangle {
        width: parent.width
        height: 1
        color: Theme.separator
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingSmall
        anchors.rightMargin: Theme.spacingSmall
        spacing: Theme.spacing

        RowLayout {
            objectName: "StatusBar.connection"
            spacing: 6
            Rectangle {
                implicitWidth: 8
                implicitHeight: 8
                radius: 4
                color: bar.stateColor(bar.connectionState)
            }
            Text {
                textFormat: Text.PlainText
                text: bar.connectionText.length ? bar.connectionText : bar.stateName(bar.connectionState)
                font: Theme.fontSmall
                color: Theme.text
                elide: Text.ElideRight
                Layout.maximumWidth: 260
            }
            Accessible.role: Accessible.StaticText
            Accessible.name: bar.stateName(bar.connectionState) + (bar.connectionText.length ? ", " + bar.connectionText : "")
        }
        Text {
            objectName: "StatusBar.scene"
            visible: bar.sceneText.length > 0
            textFormat: Text.PlainText
            text: bar.sceneText
            font: Theme.fontSmall
            color: Theme.textMuted
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
        Text {
            objectName: "StatusBar.mode"
            visible: bar.modeText.length > 0
            textFormat: Text.PlainText
            text: bar.modeText
            font: Theme.fontSmall
            color: Theme.textMuted
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
        Item { Layout.fillWidth: true }
        RowLayout {
            id: right
            spacing: Theme.spacing
        }
    }

    Accessible.role: Accessible.StatusBar
    Accessible.name: qsTr("Status")
}
