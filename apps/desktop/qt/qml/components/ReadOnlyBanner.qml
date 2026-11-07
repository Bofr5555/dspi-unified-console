import QtQuick 2.15

// The device's firmware is older or newer than this Console writes to: its settings
// show, its controls are off (bridge.writable), and this says so and offers the way
// out. WIRE-FORMAT-COMPAT.md section 4.4.
Rectangle {
    id: banner
    objectName: "readOnlyBanner"
    signal updateRequested()
    readonly property bool tooNew: bridge.compat === 3   // COMPAT_FIRMWARE_TOO_NEW

    visible: bridge.compat >= 2
    height: visible ? Math.max(message.implicitHeight + 16, 40) : 0
    radius: 8
    color: isMacOS ? MacColors.opacity(MacColors.orange, 0.12) : Qt.rgba(1, 0.62, 0.04, 0.12)
    border.color: isMacOS ? "transparent" : Qt.rgba(1, 0.62, 0.04, 0.45)

    Icon {
        id: warning
        x: 12
        anchors.verticalCenter: parent.verticalCenter
        name: "warning"
        size: 15
        color: isMacOS ? MacColors.orange : "#ff9f0a"
    }
    Text {
        id: message
        objectName: "readOnlyMessage"
        anchors.left: warning.right
        anchors.leftMargin: 9
        anchors.right: action.left
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        color: isMacOS ? MacColors.label : "white"
        text: bridge.compatMessage
    }
    Rectangle {
        id: action
        objectName: "readOnlyAction"
        property alias text: label.text
        anchors.right: parent.right
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        width: label.implicitWidth + 20
        height: 24
        radius: 7
        color: isMacOS ? (actionMouse.pressed ? MacColors.opacity(MacColors.control, 1.6) : MacColors.control) : actionMouse.pressed ? Qt.rgba(1, 1, 1, 0.13) : actionMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
        border.color: isMacOS ? "transparent" : Qt.rgba(1, 1, 1, 0.18)
        Text {
            id: label
            anchors.centerIn: parent
            text: banner.tooNew ? "Details…" : "Update…"
            font.pixelSize: 12
            color: isMacOS ? MacColors.label : "white"
        }
        MouseArea {
            id: actionMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: banner.updateRequested()
        }
    }
}
