import QtQuick 2.15

// WP DL3: registered through Registrar in DSPiBridge.cpp. A delay the core refused
// (past this device's line, say), in the core's words; gone after an accepted
// delay, a device change or Dismiss.
Item {
    id: banner
    objectName: "delayRefusalBanner"
    property bool shown: bridge.delayRefusal !== ""
    implicitHeight: 44

    Rectangle {
        anchors.fill: parent
        radius: 8
        color: Qt.rgba(1, 0.62, 0.04, 0.16)
        border.color: Qt.rgba(1, 0.62, 0.04, 0.5)
        Text {
            objectName: "delayRefusalBannerText"
            anchors { left: parent.left; leftMargin: 12; right: dismiss.left; rightMargin: 12; verticalCenter: parent.verticalCenter }
            text: qsTr("Delay not changed: %1").arg(bridge.delayRefusal)
            textFormat: Text.PlainText
            color: "white"
            font.pixelSize: 12
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
        }
        AppButton {
            id: dismiss
            objectName: "delayRefusalBannerDismiss"
            anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
            text: qsTr("Dismiss")
            onClicked: bridge.dismissDelayRefusal()
        }
    }
}
