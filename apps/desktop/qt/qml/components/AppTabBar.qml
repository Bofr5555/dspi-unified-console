import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A segmented tab bar (the Mac's segmented Picker: PEQ / XO, the settings
// pages' sections). Fill it with AppTabButton items.
TabBar {
    id: bar
    property string label
    spacing: 0
    font: Theme.fontBody
    background: Rectangle {
        radius: Theme.radiusSmall
        color: Theme.control
        border.width: 1
        border.color: Theme.border
    }

    Accessible.role: Accessible.PageTabList
    Accessible.name: label
}
