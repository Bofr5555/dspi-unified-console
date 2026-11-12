import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Window
import UConsole

// The "Updating Database" window while the AutoEQ database is rebuilt from
// GitHub (AutoEQRebuildProgressView, 340 wide): the core's progress and status
// line, a tick when it is done. Cancel stops at the next batch and writes
// nothing; closing the window does the same.
Window {
    id: window
    objectName: "AutoEqRebuild"
    title: qsTr("Updating Database")
    // The Mac's 340×180, and a row for Cancel
    width: 340
    height: 220
    minimumWidth: 340
    maximumWidth: 340
    minimumHeight: 220
    maximumHeight: 220
    flags: Qt.Dialog | Qt.CustomizeWindowHint | Qt.WindowTitleHint
    modality: Qt.ApplicationModal
    color: Theme.window
    visible: AutoEqController.rebuilding

    onClosing: close => {
        close.accepted = false
        AutoEqController.cancelRebuild()
    }

    Column {
        id: content
        anchors.centerIn: parent
        width: 280
        spacing: Theme.spacing

        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            name: "download"
            size: 40
            color: Theme.accentText
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Rebuilding AutoEQ Database")
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Rectangle {
            id: track
            objectName: "AutoEqRebuild.progress"
            property real value: AutoEqController.rebuildProgress
            width: parent.width
            height: 6
            radius: 3
            color: Theme.control
            border.color: Theme.border
            Accessible.role: Accessible.ProgressBar
            Accessible.name: qsTr("Rebuilding AutoEQ Database")
            Accessible.description: Math.round(value * 100) + " %"
            Rectangle {
                width: track.width * track.value
                height: track.height
                radius: 3
                color: Theme.accent
                Behavior on width { NumberAnimation { duration: 150 } }
            }
        }
        Text {
            objectName: "AutoEqRebuild.status"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: AutoEqController.rebuildStatus
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideMiddle
        }
        Item {
            width: parent.width
            height: Theme.controlHeight + 4
            Icon {
                anchors.centerIn: parent
                visible: AutoEqController.rebuildDone
                name: "circle-check"
                size: 24
                color: Theme.success
                accessibleName: qsTr("Done")
            }
            AppButton {
                objectName: "AutoEqRebuild.cancel"
                anchors.centerIn: parent
                visible: !AutoEqController.rebuildDone
                text: qsTr("Cancel")
                onClicked: AutoEqController.cancelRebuild()
            }
        }
    }
}
