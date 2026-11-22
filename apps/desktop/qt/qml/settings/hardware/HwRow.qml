import QtQuick
import QtQuick.Layouts
import UConsole

// A hardware row (the Mac's HStack rows on these pages): the title with a
// caption under it on the left, its control on the right.
RowLayout {
    id: row
    property string title
    property string caption
    property color captionColor: Theme.textMuted
    default property alias control: slot.data

    Layout.fillWidth: true
    spacing: Theme.spacing

    ColumnLayout {
        spacing: 1
        Layout.fillWidth: true
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: row.title
            font: Theme.fontBody
            color: row.enabled ? Theme.text : Theme.textDisabled
            wrapMode: Text.Wrap
        }
        Text {
            Layout.fillWidth: true
            visible: text.length > 0
            textFormat: Text.PlainText
            text: row.caption
            font: Theme.fontSmall
            color: row.enabled ? row.captionColor : Theme.textDisabled
            wrapMode: Text.Wrap
        }
    }
    Item {
        id: slot
        Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
    }
}
