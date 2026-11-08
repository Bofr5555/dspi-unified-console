import QtQuick 2.15

// Registered through Registrar in DemoController.cpp. With nothing connected, the way
// into the demo. In the demo, the label: always there, and with the way out when
// there is one.
Item {
    id: banner
    objectName: "demoBanner"
    property bool shown: demo.active || demo.offered
    implicitHeight: 44

    Rectangle {
        anchors.fill: parent
        radius: 8
        color: demo.active ? Qt.rgba(0.04, 0.49, 1, 0.18) : Qt.rgba(1, 1, 1, 0.06)
        border.color: demo.active ? Qt.rgba(0.04, 0.49, 1, 0.5) : Qt.rgba(1, 1, 1, 0.18)
        Text {
            objectName: "demoBannerText"
            anchors { left: parent.left; leftMargin: 12; right: buttons.left; rightMargin: 12; verticalCenter: parent.verticalCenter }
            text: demo.active
                  ? (demo.leavable && bridge.availableSerials.length > 0 ? qsTr("Demo: a simulated DSPi. Nothing is sent to a real device. Leave the demo to use the DSPi that is connected.")
                                                                         : qsTr("Demo: a simulated DSPi. Nothing is sent to a real device."))
                  : demo.failure !== "" ? demo.failure
                  : qsTr("No DSPi connected. The demo shows the app on a simulated one.")
            color: "white"
            font.pixelSize: 12
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
        }
        Row {
            id: buttons
            anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
            AppButton {
                objectName: "demoEnter"
                visible: demo.offered
                text: qsTr("Open Demo")
                onClicked: demo.enter()
            }
            AppButton {
                objectName: "demoLeave"
                visible: demo.leavable
                text: qsTr("Leave Demo")
                onClicked: demo.leave()
            }
        }
    }
}
