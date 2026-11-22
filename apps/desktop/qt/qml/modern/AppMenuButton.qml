import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The app button, "uConsole ▾" (MODLAY 3.1): Modern has no menu
// bar. Its menu holds the few global actions (-- WP LAY3b -- MenuTable's place
// appMenu); every other one is where it acts. The shortcuts are the shell's
// (TableShortcuts); the rows only show them.
AbstractButton {
    id: button
    objectName: "AppMenuButton"
    property Window window: null

    text: AppIdentity.displayName
    implicitHeight: 32
    implicitWidth: row.implicitWidth + 16
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    onClicked: menu.popupFor(({}), button, 0, button.height + 2)

    contentItem: Item {
        Row {
            id: row
            anchors.centerIn: parent
            spacing: 6
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: "uConsole"
                font: Theme.fontBody
                color: Theme.text
            }
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                name: "chevron-down"
                size: 12
                color: Theme.textMuted
            }
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.down ? Theme.controlPressed : button.hovered ? Theme.controlHover : Theme.control
        border.width: button.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }
    Accessible.role: Accessible.ButtonMenu
    Accessible.name: text

    ActionMenu {
        id: menu
        objectName: "AppMenu"
        place: "appMenu"
        handlers: ({ "app.classicLayout": { run: () => LayoutController.layout = "classic" } })
    }
}
