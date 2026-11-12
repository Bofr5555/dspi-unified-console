import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A thin scroll bar that shows while scrolling or hovered:
//     ScrollBar.vertical: AppScrollBar {}
ScrollBar {
    id: bar
    hoverEnabled: true
    minimumSize: 0.1
    padding: 2
    policy: size < 1 ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff

    contentItem: Rectangle {
        implicitWidth: bar.hovered || bar.pressed ? 8 : 5
        implicitHeight: bar.hovered || bar.pressed ? 8 : 5
        radius: Math.min(width, height) / 2
        color: bar.pressed ? Theme.textMuted : Theme.border
        opacity: bar.active || bar.hovered ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 200 } }
    }
    background: Item {}

    Accessible.role: Accessible.ScrollBar
    Accessible.name: orientation === Qt.Vertical ? qsTr("Vertical scroll bar") : qsTr("Horizontal scroll bar")
}
