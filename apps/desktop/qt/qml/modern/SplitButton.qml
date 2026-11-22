import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3 -- A button with a menu beside it (Save to device ▾): the main
// part runs `action`, the ▾ lists `actions`, both MenuTable ids (their texts,
// their enabled state and, on the menu's rows, their shortcuts). WP LAY3b fills
// the menus from MenuTable's placements.
Row {
    id: split
    property string action: ""
    property var actions: []
    property string text: ""
    property string iconName: ""
    property string toolTip: ""
    property bool primary: false
    // A count after the text, 0 for none
    property int count: 0
    property Window window: null

    function can(id) {
        const row = MenuTable.shortcutRows().find(r => r.id === id)
        return MenuTable.available(id) && (!row || !row.enabled || row.enabled())
    }

    spacing: 1

    AppButton {
        objectName: split.objectName + ".button"
        height: 28
        width: text.length ? implicitWidth : 32
        text: split.count > 0 ? (split.text.length ? split.text + "  " + split.count : String(split.count)) : split.text
        iconName: split.iconName
        primary: split.primary
        toolTip: split.toolTip
        enabled: split.can(split.action)
        onClicked: MenuTable.trigger(split.window, split.action)
        Accessible.name: split.toolTip.length ? split.toolTip : split.text
    }
    AppButton {
        id: more
        objectName: split.objectName + ".more"
        height: 28
        width: 24
        leftPadding: 4
        rightPadding: 4
        iconName: "chevron-down"
        primary: split.primary
        visible: split.actions.length > 0
        //: %1 is a button's text, "Save to device"
        toolTip: qsTr("More for %1").arg(split.toolTip.length ? split.toolTip : split.text)
        onClicked: menu.popup(more, more.width - menu.width, more.height + 2)
    }
    AppMenu {
        id: menu
        objectName: split.objectName + ".menu"
        Instantiator {
            model: split.actions
            delegate: AppMenuItem {
                required property string modelData
                objectName: split.objectName + "." + modelData
                text: MenuTable.textOf(modelData, false)
                enabled: split.can(modelData)
                shortcutText: MenuTable.shortcutOf(modelData)
                onTriggered: MenuTable.trigger(split.window, modelData)
            }
            onObjectAdded: (index, object) => menu.insertItem(index, object)
            onObjectRemoved: (index, object) => menu.removeItem(object)
        }
    }
}
