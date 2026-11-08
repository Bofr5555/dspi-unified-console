import QtQuick 2.15

// Registered through Registrar in LinkController.cpp. The owner's rule for a lost
// Link connection: say so at once, with the way out. CG10's wording. The mirror
// stays on screen behind this and writes fail in the core; nothing here waits.
Item {
    id: banner
    objectName: "linkBanner"
    readonly property bool lost: linkSession.phase === "reconnecting"
    readonly property bool warned: linkSession.tokenWarning !== ""
    property bool shown: lost || warned
    implicitHeight: lostRow.visible && warnRow.visible ? 92 : 44

    Column {
        anchors.fill: parent
        spacing: 4

        Rectangle {
            id: lostRow
            objectName: "linkLostRow"
            visible: banner.lost
            width: parent.width
            height: 44
            radius: 8
            color: Qt.rgba(1, 0.6, 0, 0.18)
            border.color: Qt.rgba(1, 0.6, 0, 0.45)
            Text {
                objectName: "linkLostText"
                anchors { left: parent.left; leftMargin: 12; right: abort.left; rightMargin: 12; verticalCenter: parent.verticalCenter }
                text: linkSession.phaseText
                color: "white"
                font.pixelSize: 13
                elide: Text.ElideRight
            }
            AppButton {
                id: abort
                objectName: "linkLostAbort"
                anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
                text: linkSession.actionText
                onClicked: linkSession.closeSession()
            }
        }

        Rectangle {
            id: warnRow
            objectName: "linkTokenWarnRow"
            visible: banner.warned
            width: parent.width
            height: 44
            radius: 8
            color: Qt.rgba(1, 1, 1, 0.06)
            border.color: Qt.rgba(1, 1, 1, 0.18)
            Text {
                anchors { left: parent.left; leftMargin: 12; right: dismiss.left; rightMargin: 12; verticalCenter: parent.verticalCenter }
                text: linkSession.tokenWarning
                color: "white"
                font.pixelSize: 12
                wrapMode: Text.WordWrap
                maximumLineCount: 2
                elide: Text.ElideRight
            }
            AppButton {
                id: dismiss
                anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
                text: qsTr("OK")
                onClicked: linkSession.dismissTokenWarning()
            }
        }
    }
}
