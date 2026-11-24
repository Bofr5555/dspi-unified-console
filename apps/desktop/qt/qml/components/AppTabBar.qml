import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A segmented tab bar (the Mac's segmented Picker: PEQ / XO, the settings
// pages' sections). Fill it with AppTabButton items.
// -- WP REVDFIX -- (REVD D2) Modern: ToolSegments' look (DESIGN-LANGUAGE §7):
// one bar in the control tone with no outline, the chosen tab in the
// selection tone with accent text (never an accent fill), as wide as its tabs
// at the left of the room it gets. Classic keeps its bordered bar.
TabBar {
    id: bar
    property string label
    readonly property bool modern: LayoutController.modern
    spacing: 0
    padding: modern ? 2 : 0
    font: modern ? Theme.fontSmall : Theme.fontBody
    Layout.maximumWidth: modern ? implicitWidth : Number.POSITIVE_INFINITY
    background: Rectangle {
        radius: bar.modern ? Theme.radiusControl : Theme.radiusSmall
        color: Theme.control
        border.width: bar.modern ? 0 : 1
        border.color: Theme.border
    }

    Accessible.role: Accessible.PageTabList
    Accessible.name: label
}
