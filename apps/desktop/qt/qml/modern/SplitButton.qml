import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3 -- A button with a menu beside it (Save to device ▾): the main
// part runs `action`, a MenuTable id. -- WP LAY3b -- The ▾ is MenuTable's
// `place` (ActionMenu, with `handlers` for rows that are not the menus').
// -- WP UA3 -- `countToolTip` says what the count counts; `actionEnabled`
// greys the main part beyond what MenuTable says (no device); `leadRows`
// are rows above the place's own, with a line under them:
// [{ name, text, enabled, run }], each "<name>.menu.<row name>".
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
    // -- WP UA3 --
    property string countToolTip: ""
    property bool actionEnabled: true
    property var leadRows: []

    spacing: 1

    AppButton {
        objectName: split.objectName + ".button"
        height: 28
        width: text.length ? implicitWidth : 32
        text: split.count > 0 ? (split.text.length ? split.text + "  " + split.count : String(split.count)) : split.text
        iconName: split.iconName
        primary: split.primary
        toolTip: split.count > 0 && split.countToolTip.length ? split.countToolTip : split.toolTip   // -- WP UA3 --
        enabled: MenuTable.can(split.action) && split.actionEnabled
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
        // -- WP UA3 -- ActionMenu has made its rows by now: the lead rows go on top
        property var leads: []
        onAboutToShow: {
            for (const o of leads) o.destroy()
            leads = []
            if (split.leadRows.length === 0) return
            let at = 0
            for (const r of split.leadRows) {
                const run = r.run
                const item = leadItem.createObject(null, {
                    objectName: split.objectName + ".menu." + r.name, text: r.text, enabled: r.enabled !== false
                })
                item.triggered.connect(() => run())
                insertItem(at++, item)
                leads.push(item)
                widest = Math.max(widest, item.implicitWidth)
            }
            if (count > at) {
                const line = leadLine.createObject(null)
                insertItem(at, line)
                leads.push(line)
            }
        }
    }
    Component { id: leadItem; AppMenuItem {} }
    Component {
        id: leadLine
        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.separator
            }
        }
    }
}
