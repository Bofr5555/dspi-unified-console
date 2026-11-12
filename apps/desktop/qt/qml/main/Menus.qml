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

    function triggerRow(id) { MenuTable.trigger(window, id) }

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
        fillFavorites()   // -- WP QP15 --
    }

    // -- WP QP15 -- AutoEQ > Favorite Profiles: a row per favourite above Clear
    // Favorites, which works only with something to clear; "No favorites yet"
    // while there is none. The rows follow AutoEqController.favorites.
    property Menu favoritesMenu: null
    property var favoriteRows: []
    function menuHolding(menu, name) {
        for (let i = 0; i < menu.count; ++i) {
            const item = menu.itemAt(i)
            if (item && item.objectName === name) return menu
            if (item && item.subMenu) {
                const found = menuHolding(item.subMenu, name)
                if (found) return found
            }
        }
        return null
    }
    function fillFavorites() {
        if (!favoritesMenu) {
            for (let i = 0; i < bar.count && !favoritesMenu; ++i) favoritesMenu = menuHolding(bar.menuAt(i), "Menu.autoeq.clearFavorites")
            if (!favoritesMenu) return
            const clear = favoritesMenu.itemAt(favoritesMenu.count - 1)
            if (clear && clear.action) clear.action.enabled = Qt.binding(() => AutoEqController.favorites.length > 0)
            favoritesMenu.insertItem(0, separatorComponent.createObject(bar))
        }
        for (const row of favoriteRows) {
            favoritesMenu.removeItem(row)
            row.objectName = ""   // gone now, though destroyed later
            row.destroy()
        }
        const rows = []
        const favorites = AutoEqController.favorites
        favorites.forEach((f, i) => {
            const item = itemComponent.createObject(bar, { text: f.title, objectName: "Menu.autoeq.favorite." + i })
            item.Accessible.name = f.title
            item.enabled = Qt.binding(() => AutoEqController.canApply)
            const id = f.id
            item.triggered.connect(() => AutoEqController.apply(id))
            rows.push(item)
        })
        if (favorites.length === 0) {
            const none = itemComponent.createObject(bar, { text: qsTr("No favorites yet"), enabled: false,
                                                           objectName: "Menu.autoeq.noFavorites" })
            rows.push(none)
        }
        rows.forEach((r, i) => favoritesMenu.insertItem(i, r))
        favoriteRows = rows
    }
    Connections {
        target: AutoEqController
        function onFavoritesChanged() { bar.fillFavorites() }
    }
}
