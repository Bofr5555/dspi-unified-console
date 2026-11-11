import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The main window's menu bar, built from MenuTable. Each row is an Action
// (its text and shortcut) whose trigger goes to triggerRow(); every menu item
// is named "Menu.<action id>" for the smoke driver.
AppMenuBar {
    id: bar
    objectName: "Menu.bar"
    required property Window window

    // Edit's rows act on the focused text field first, as the Mac sends them
    // to the first responder; otherwise they go to whoever handles the id (the
    // channel clipboard).
    readonly property var textEdits: ({
        "edit.cut": "cut",
        "edit.copy": "copy",
        "edit.paste": "paste",
        "edit.selectAll": "selectAll",
        "edit.delete": "delete"
    })

    function triggerRow(id) {
        const target = window.activeFocusItem
        const op = textEdits[id]
        if (op && target && target.selectionStart !== undefined && typeof target.selectAll === "function") {
            if (op === "delete") target.remove(target.selectionStart, target.selectionEnd)
            else target[op]()
            return
        }
        Shell.trigger(id)
    }

    Component { id: menuComponent; AppMenu {} }
    Component { id: itemComponent; AppMenuItem {} }
    Component { id: actionComponent; Action {} }
    Component {
        id: separatorComponent
        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.separator
            }
        }
    }

    function addRows(menu, rows) {
        for (const row of rows) {
            if (row.separator) {
                menu.addItem(separatorComponent.createObject(bar))
            } else if (row.rows) {
                const sub = menuComponent.createObject(menu, { title: row.submenu })
                addRows(sub, row.rows)
                menu.addMenu(sub)
            } else {
                const id = row.id
                const shown = row.optional ? Shell.handles(id) : true
                const action = actionComponent.createObject(menu, { text: row.text, shortcut: row.shortcut || "" })
                action.triggered.connect(() => bar.triggerRow(id))
                if (row.platformHidden) {
                    const hidden = row.platformHidden
                    action.enabled = Qt.binding(() => DeviceModel.platformName !== hidden)
                }
                const item = itemComponent.createObject(bar, { action: action, objectName: "Menu." + id })
                item.Accessible.name = row.text
                if (!shown) item.visible = false
                else if (row.platformHidden) item.visible = Qt.binding(() => action.enabled)
                // A hidden row takes no room
                item.height = Qt.binding(() => item.visible ? item.implicitHeight : 0)
                menu.addItem(item)
            }
        }
    }

    Component.onCompleted: {
        for (const m of MenuTable.menus) {
            const menu = menuComponent.createObject(bar, { title: m.title })
            addRows(menu, m.rows)
            bar.addMenu(menu)
        }
    }
}
