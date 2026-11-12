import QtQuick
import QtQuick.Layouts
import UConsole

// A switch row: the title and a caption under it on the left, the switch on
// the right (the Mac's Toggle with a VStack label). `toggled(on)` fires on a
// person's change only.
RowLayout {
    id: row
    property string title
    property string caption
    property bool checked
    property alias control: toggle
    signal toggled(bool on)

    Layout.fillWidth: true
    spacing: Theme.spacing

    ColumnLayout {
        Layout.fillWidth: true
        spacing: 2
        Text {
            Layout.fillWidth: true
            text: row.title
            font: Theme.fontBody
            color: row.enabled ? Theme.text : Theme.textDisabled
            wrapMode: Text.Wrap
        }
        Text {
            visible: row.caption.length > 0
            Layout.fillWidth: true
            text: row.caption
            font: Theme.fontSmall
            color: row.enabled ? Theme.textMuted : Theme.textDisabled
            wrapMode: Text.Wrap
        }
    }
    AppSwitch {
        id: toggle
        objectName: row.objectName.length ? row.objectName + ".switch" : ""
        checked: row.checked
        onToggled: row.toggled(checked)
        Accessible.name: row.title
        Accessible.description: row.caption
    }
}
