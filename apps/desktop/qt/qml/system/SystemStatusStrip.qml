import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LKUI1 -- A compact status line under the menu bar while a hub's device
// is open and the hub has an audio link installation: the link state of its
// units in one line (the dot's colour is its meaning), the last scene recalled,
// the system mute and the alarms still on. System... opens the window.
Rectangle {
    id: strip
    objectName: "SystemStatusStrip"
    property string shownName: summary.text
    visible: SystemController.available && SystemController.installed
    implicitHeight: Theme.statusBarHeight
    color: Theme.sidebar
    Accessible.role: Accessible.StatusBar
    Accessible.name: shownName

    function healthColor(h) {
        if (h === "ok") return Theme.success
        if (h === "warning") return Theme.warning
        if (h === "problem") return Theme.dangerText
        return Theme.offline
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.separator
    }
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: Theme.spacingSmall
        spacing: Theme.spacing
        Rectangle {
            implicitWidth: 8
            implicitHeight: 8
            radius: 4
            color: strip.healthColor(SystemController.health)
        }
        Text {
            id: summary
            objectName: "SystemStatusStrip.summary"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: SystemController.summary
                  + (SystemController.recalledScene.length ? "   ·   " + qsTr("Scene: %1").arg(SystemController.recalledScene) : "")
                  + (SystemController.systemMute === true ? "   ·   " + qsTr("System mute on") : "")
            font: Theme.fontSmall
            color: Theme.text
            elide: Text.ElideRight
        }
        Text {
            visible: SystemController.alarms.length > 0
            textFormat: Text.PlainText
            text: qsTr("%n alarm(s)", "", SystemController.alarms.length)
            font: Theme.fontSmall
            color: Theme.alarmRaised
        }
        Text {
            objectName: "SystemStatusStrip.open"
            text: qsTr("System…")
            font: Theme.fontSmall
            color: Theme.accentText
            Accessible.role: Accessible.Link
            Accessible.name: qsTr("Open the System window")
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: Shell.trigger("window.system")
            }
        }
    }
}
