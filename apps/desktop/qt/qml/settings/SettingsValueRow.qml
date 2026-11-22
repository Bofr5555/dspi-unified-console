import QtQuick
import QtQuick.Layouts
import UConsole

// A read-only row: the label on the left, its value on the right, selectable
// for copying (the Mac's status rows).
RowLayout {
    id: row
    property string title
    property string value
    property bool mono: false

    Layout.fillWidth: true
    spacing: Theme.spacing

    Text {
        textFormat: Text.PlainText
        text: row.title
        font: Theme.fontBody
        color: Theme.text
    }
    TextEdit {
        objectName: row.objectName.length ? row.objectName + ".value" : ""
        Layout.fillWidth: true
        text: row.value
        readOnly: true
        selectByMouse: true
        font: row.mono ? Theme.fontMono : Theme.fontBody
        color: Theme.textMuted
        selectionColor: Theme.textSelection
        selectedTextColor: Theme.text
        horizontalAlignment: TextEdit.AlignRight
        wrapMode: TextEdit.Wrap
        Accessible.role: Accessible.StaticText
        Accessible.name: row.title
        Accessible.description: row.value
    }
}
