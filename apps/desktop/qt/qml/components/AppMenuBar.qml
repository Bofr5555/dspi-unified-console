import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The in-window menu bar under the system's window frame.
MenuBar {
    id: bar
    font: Theme.fontBody
    delegate: MenuBarItem {
        id: item
        font: bar.font
        contentItem: Text {
            text: item.text.replace("&", "")
            font: item.font
            color: Theme.text
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            color: item.highlighted ? Theme.controlHover : "transparent"
            radius: Theme.radiusSmall
        }
    }
    background: Rectangle {
        color: Theme.window
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.separator
        }
    }
}
