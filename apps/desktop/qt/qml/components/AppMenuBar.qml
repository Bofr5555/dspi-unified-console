import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The in-window menu bar under the system's window frame: the menu titles on
// the window colour, with a hairline along its foot.
MenuBar {
    id: bar
    font: Theme.fontBody

    // One menu title. Mnemonics are not drawn, so their marker is dropped.
    component Title: MenuBarItem {
        id: title
        font: bar.font
        background: Rectangle {
            radius: Theme.radiusSmall
            color: title.highlighted ? Theme.controlHover : "transparent"
        }
        contentItem: Text {
            textFormat: Text.PlainText
            text: title.text.replace("&", "")
            color: Theme.text
            font: title.font
            horizontalAlignment: Qt.AlignHCenter
            verticalAlignment: Qt.AlignVCenter
        }
    }

    delegate: Title {}
    background: Rectangle {
        color: Theme.window
        Rectangle {
            color: Theme.separator
            implicitHeight: 1
            y: parent.height - implicitHeight
            implicitWidth: parent.width
        }
    }
}
