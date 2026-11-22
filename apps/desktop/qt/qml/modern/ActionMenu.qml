import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3b -- A menu of one of MenuTable's places (the app button's, Save ▾,
// a channel's right-click menu, ...): its groups with a line between two. The
// rows are made each time it opens (popupFor()), for its `target`, so a hidden
// row takes no room and a group without rows no line.
//
// A menu row runs by MenuTable.trigger(); a row on one thing (itemRows) by its
// entry in `handlers`, which may also overrule a menu row's:
//   "<id>": {
//     run(target, value)   what it does; `value` is a dynamic row's
//     shown(target)        false leaves it out
//     enabled(target)      false greys it
//     on(target)           it is on: the row's `on` text
//     text(target)         another text
//     why(target)          why it is greyed, "" for none: a line under it
//     keys(target)         false hides the shortcut (it acts on something else)
//     rows(target)         a submenu's or a dynamic row's own rows:
//                          [{ name, text, value, checked, enabled, detail }]
//   }
// Object names: "<menu>.<action id>", a dynamic row's "<menu>.<action id>.<name>".
AppMenu {
    id: menu
    property string place: ""
    property var handlers: ({})
    // What a right-click menu is about ({ channel: 3 }), given with popupFor()
    property var target: ({})
    property Window window: null
    property var made: []
    // Where rows are made: a child of the menu's own content is a row at once
    readonly property Item nursery: Item {}

    implicitWidth: Math.min(460, Math.max(240, widest))
    property real widest: 0

    // Opens for `what`, at a point of `item`. Its parent is the window's
    // content, not the item: a list may drop the item while the menu is open
    function popupFor(what, item, x, y) {
        target = what
        window = item.Window.window
        rebuild()
        const root = window ? window.contentItem : item
        const at = item.mapToItem(root, x, y)
        popup(root, at.x, at.y)
    }
    // Under `button`, its right edges together
    function dropFrom(button) {
        rebuild()
        popupFor(({}), button, button.width - implicitWidth, button.height + 2)
    }

    function rebuild() {
        while (count > 0) {
            if (itemAt(0).subMenu) takeMenu(0)   // with the menu's own item for it
            else takeItem(0)
        }
        for (const o of made) {
            o.objectName = ""   // gone now, though destroyed later
            o.destroy()
        }
        made = []
        widest = 0
        const groups = MenuTable.placeRows(place)
        for (const rows of groups) {
            const items = []
            for (const r of rows) addRow(r, items)
            if (items.length === 0) continue
            if (count > 0) keep(separatorComponent.createObject(menu.nursery))
            for (const i of items) {
                if (i.subMenuOf) {
                    addMenu(i.subMenuOf)
                    itemAt(count - 1).enabled = Qt.binding(i.usable)
                } else {
                    addItem(i)
                }
                if (i.implicitWidth > 0) widest = Math.max(widest, i.implicitWidth)
            }
        }
    }
    function keep(separator) {
        made.push(separator)
        addItem(separator)
    }

    function addRow(r, items) {
        const h = handlers[r.id] || ({})
        if (r.optional && !Shell.handles(r.id)) return
        if (r.platformHidden && DeviceModel.platformName === r.platformHidden) return
        if (h.shown && !h.shown(target)) return
        const name = menu.objectName + "." + r.id
        // -- WP LAY7 -- (review RC, RC9) read in the rows' bindings: what begins
        // while the menu is open (a pin, read-only) greys them too
        const usable = () => MenuTable.can(r.id) && (!h.enabled || h.enabled(menu.target)) && (!h.why || h.why(menu.target) === "")
        if (r.dynamic || r.submenu) {
            const rows = h.rows ? h.rows(target) : []
            if (rows.length === 0) return
            let sub = null
            if (r.submenu) {
                sub = submenuComponent.createObject(menu, { objectName: name, title: h.text ? h.text(target) : r.text })
                made.push(sub)
                items.push({ subMenuOf: sub, usable: usable })
            } else {
                items.push(made[made.push(headingComponent.createObject(menu.nursery, { objectName: name, text: h.text ? h.text(target) : r.text })) - 1])
            }
            for (const e of rows) {
                const value = e.value
                const item = itemComponent.createObject(menu.nursery, {
                    objectName: name + "." + e.name, text: e.text, shortcutText: e.detail || "",
                    checkable: e.checked !== undefined
                })
                item.enabled = Qt.binding(() => e.enabled !== false && usable())
                // A checkable row would switch itself over
                if (e.checked !== undefined) item.checked = e.checked
                item.triggered.connect(() => h.run(menu.target, value))
                made.push(item)
                if (sub) sub.addItem(item)
                else items.push(item)
            }
            return
        }
        const id = r.id
        const keys = MenuTable.shortcutOf(r.of || id)
        const item = itemComponent.createObject(menu.nursery, {
            objectName: name,
            text: h.text ? h.text(target) : h.on && h.on(target) ? r.on : r.text,
            shortcutText: !h.keys || h.keys(target) ? keys : ""
        })
        item.enabled = Qt.binding(usable)
        if (h.run) item.triggered.connect(() => h.run(menu.target))
        else item.triggered.connect(() => MenuTable.trigger(menu.window, r.of || id))
        made.push(item)
        items.push(item)
        if (h.why) {
            const reason = reasonComponent.createObject(menu.nursery, { objectName: name + ".why" })
            reason.text = Qt.binding(() => h.why(menu.target))
            made.push(reason)
            items.push(reason)
            item.Accessible.description = Qt.binding(() => h.why(menu.target))
        }
    }

    Component { id: itemComponent; AppMenuItem {} }
    Component { id: submenuComponent; AppMenu {} }
    Component {
        id: separatorComponent
        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.separator
            }
        }
    }
    // Over dynamic rows
    Component {
        id: headingComponent
        AppMenuItem {
            enabled: false
            font: Theme.fontHeading
        }
    }
    // Why the row above is greyed, wrapped; no room with nothing to say
    Component {
        id: reasonComponent
        MenuItem {
            id: reason
            enabled: false
            visible: text.length > 0
            implicitWidth: 0
            implicitHeight: visible ? reasonText.implicitHeight + 8 : 0
            contentItem: Text {
                id: reasonText
                leftPadding: 6
                rightPadding: 6
                textFormat: Text.PlainText
                text: reason.text
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            background: null
        }
    }
}
