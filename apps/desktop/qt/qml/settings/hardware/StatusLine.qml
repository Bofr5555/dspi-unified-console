import QtQuick
import QtQuick.Layouts
import UConsole

// The outcome of the last pin change on a hardware page (the Mac's status
// row): a check or a warning sign and the line.
RowLayout {
    id: line
    property string text: HardwareController.statusText
    property bool error: HardwareController.statusError

    visible: text.length > 0
    spacing: 6
    Layout.fillWidth: true
    Icon {
        name: line.error ? "triangle-alert" : "circle-check"
        size: 12
        color: line.error ? Theme.warning : Theme.success
    }
    Text {
        objectName: line.objectName.length ? line.objectName + ".text" : ""
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: line.text
        font: Theme.fontSmall
        color: line.error ? Theme.warning : Theme.textMuted
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
