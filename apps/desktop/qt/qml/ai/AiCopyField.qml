import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- A text to put into an agent's configuration: selectable, with a
// Copy button.
RowLayout {
    id: field
    property string label
    property string text
    Layout.fillWidth: true
    spacing: Theme.spacingSmall

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: edit.implicitHeight + 12
        radius: Theme.radiusSmall
        color: Theme.control
        border.width: 1
        border.color: Theme.border
        clip: true
        TextEdit {
            id: edit
            objectName: field.objectName.length ? field.objectName + ".text" : ""
            x: 6
            y: 6
            width: parent.width - 12
            text: field.text
            textFormat: TextEdit.PlainText
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.WrapAnywhere
            font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 12 })
            color: Theme.text
            selectionColor: Theme.selection
            selectedTextColor: Theme.text
            Accessible.role: Accessible.StaticText
            Accessible.name: field.label
            Accessible.description: text
        }
    }
    IconButton {
        objectName: field.objectName.length ? field.objectName + ".copy" : ""
        Layout.alignment: Qt.AlignTop
        iconName: "copy"
        toolTip: qsTr("Copy")
        onClicked: AiController.copy(field.text)
    }
}
