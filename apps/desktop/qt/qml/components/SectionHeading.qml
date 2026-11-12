import QtQuick
import QtQuick.Layouts
import UConsole

// A section's title in a page or a sidebar ("INPUTS", "Device"), with an
// optional line under it. `caps` gives the sidebar's small capitals.
ColumnLayout {
    id: heading
    property string text
    property bool caps: true
    property bool rule: false
    spacing: 4

    Text {
        Layout.fillWidth: true
        text: heading.text
        font: heading.caps ? Theme.fontHeading : Theme.fontTitle
        color: heading.caps ? Theme.textMuted : Theme.text
        elide: Text.ElideRight
        Accessible.role: Accessible.Heading
        Accessible.name: heading.text
    }
    Rectangle {
        visible: heading.rule
        Layout.fillWidth: true
        implicitHeight: 1
        color: Theme.separator
    }
}
