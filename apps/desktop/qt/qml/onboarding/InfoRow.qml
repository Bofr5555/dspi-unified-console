import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- One line of a wizard step: an accent icon in a fixed column,
// then the text (the Mac's infoRow).
RowLayout {
    id: line
    property string iconName
    property string text

    Layout.fillWidth: true
    spacing: 10
    Icon {
        Layout.alignment: Qt.AlignTop
        Layout.preferredWidth: 20
        name: line.iconName
        color: Theme.accentText
    }
    Text {
        Layout.fillWidth: true
        text: line.text
        font: Theme.fontBody
        color: Theme.text
        wrapMode: Text.Wrap
    }
}
