import QtQuick 2.15
import QtQuick.Controls 2.15
import "../components"

// -- WP LIC1 -- One licence text built into the app (AboutController::licenceText),
// as plain, selectable text over the settings window. Esc or Close leaves it.
Popup {
    id: popup
    property string heading: ""
    property string file: ""
    readonly property string body: file !== "" ? aboutInfo.licenceText(file) : ""

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: parent ? parent.width - 48 : 600
    height: parent ? parent.height - 48 : 480
    padding: 16
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    background: Rectangle {
        radius: isMacOS ? 8 : 12
        color: isMacOS ? "#2b2b2b" : "#1e1e22"
        border.color: Qt.rgba(1, 1, 1, 0.12)
    }

    Item {
        anchors.fill: parent
        Text {
            id: title
            width: parent.width - close.width - 12
            text: popup.heading
            textFormat: Text.PlainText
            elide: Text.ElideRight
            font.pixelSize: 15
            font.weight: Font.DemiBold
            color: isMacOS ? MacColors.label : "white"
        }
        Button {
            id: close
            objectName: "licenceTextClose"
            anchors.right: parent.right
            anchors.verticalCenter: title.verticalCenter
            text: qsTr("Close")
            onClicked: popup.close()
        }
        ScrollView {
            anchors.top: title.bottom
            anchors.topMargin: 12
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            clip: true
            TextArea {
                objectName: "licenceTextBody"
                readOnly: true
                selectByMouse: true
                textFormat: TextEdit.PlainText
                wrapMode: TextEdit.Wrap
                text: popup.body !== "" ? popup.body : qsTr("This text is missing from this build.")
                font.family: "monospace"
                font.pixelSize: 11
                color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85)
                background: null
            }
        }
    }
}
