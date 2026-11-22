import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The app button, "uConsole ▾" (MODLAY 3.1): Modern has no menu
// bar. Until WP LAY3b places every action where it acts and keeps the rare ones
// here, its menu holds all of MenuTable's menus, so nothing is out of reach.
// The shortcuts are the shell's (TableShortcuts); the rows only show them.
AbstractButton {
    id: button
    objectName: "AppMenuButton"
    property Window window: null

    text: AppIdentity.displayName
    implicitHeight: 32
    implicitWidth: row.implicitWidth + 16
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    onClicked: menu.popup(button, 0, button.height + 2)

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

    AppMenu {
        id: menu
        objectName: "AppMenu"
    }
    Component { id: menuComponent; AppMenu {} }
    Component { id: itemComponent; AppMenuItem {} }
    Component {
        id: separatorComponent
        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.separator
            }
        }
    }

    function addRows(into, rows) {
        for (const r of rows) {
            if (r.separator) {
                into.addItem(separatorComponent.createObject(button))
            } else if (r.rows) {
                const sub = menuComponent.createObject(into, { title: r.submenu })
                addRows(sub, r.rows)
                into.addMenu(sub)
            } else {
                const id = r.id
                const item = itemComponent.createObject(button, { objectName: "AppMenu." + id, shortcutText: r.shortcut || "" })
                item.text = Qt.binding(() => MenuTable.textOf(id, false))
                const rowEnabled = r.enabled
                item.enabled = Qt.binding(() => MenuTable.available(id) && (!rowEnabled || rowEnabled()))
                const hiddenOn = r.platformHidden
                const shown = r.optional ? Shell.handles(id) : true
                item.visible = Qt.binding(() => shown && (!hiddenOn || DeviceModel.platformName !== hiddenOn))
                // A hidden row takes no room
                item.height = Qt.binding(() => item.visible ? item.implicitHeight : 0)
                item.triggered.connect(() => MenuTable.trigger(button.window, id))
                into.addItem(item)
            }
        }
    }
    Component.onCompleted: {
        MenuTable.menus.forEach((m, i) => {
            const sub = menuComponent.createObject(menu)
            sub.title = Qt.binding(() => MenuTable.menus[i].title.replace("&", ""))
            addRows(sub, m.rows)
            menu.addMenu(sub)
        })
        menu.addItem(separatorComponent.createObject(button))
        const classic = itemComponent.createObject(button, { objectName: "AppMenu.classicLayout" })
        classic.text = Qt.binding(() => qsTr("Switch to the Classic Layout"))
        classic.triggered.connect(() => LayoutController.layout = "classic")
        menu.addItem(classic)
    }
}
