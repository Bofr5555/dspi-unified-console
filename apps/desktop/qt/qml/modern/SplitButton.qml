import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3 -- A button with a menu beside it (Save to device ▾): the main
// part runs `action`, a MenuTable id. -- WP LAY3b -- The ▾ is MenuTable's
// `place` (ActionMenu, with `handlers` for rows that are not the menus').
Row {
    id: split
    property string action: ""
    property string place: ""
    property var handlers: ({})
    property string text: ""
    property string iconName: ""
    property string toolTip: ""
    property bool primary: false
    // A count after the text, 0 for none
    property int count: 0
    property Window window: null

    spacing: 1

    AppButton {
        objectName: split.objectName + ".button"
        height: 28
        width: text.length ? implicitWidth : 32
        text: split.count > 0 ? (split.text.length ? split.text + "  " + split.count : String(split.count)) : split.text
        iconName: split.iconName
        primary: split.primary
        toolTip: split.toolTip
        enabled: MenuTable.can(split.action)
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
        //: %1 is a button's text, "Save to device"
        toolTip: qsTr("More for %1").arg(split.toolTip.length ? split.toolTip : split.text)
        onClicked: menu.dropFrom(more)
    }
    ActionMenu {
        id: menu
        objectName: split.objectName + ".menu"
        place: split.place
        handlers: split.handlers
    }
}
