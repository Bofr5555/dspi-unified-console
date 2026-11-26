import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The sections (SectionTable): a slim rail on the left (variant
// B), or tabs across the top (variant A) with `horizontal`; Devices and
// Settings at the far end. LayoutController.arrangement.navigation says which.
// `header` is room at the rail's top for the device tabs.
// -- WP UA5 -- One Tab stop: the section on show. The arrow keys (Up/Left,
// Down/Right), Home and End go to another section and show it; a click leaves
// no focus ring behind (the focus follows it without one).
Rectangle {
    id: rail
    objectName: "NavRail"
    property bool horizontal: false
    property string current: ""
    property alias header: headerSlot.data
    property bool headerShown: false
    signal chosen(string section)

    // -- WP UA5 -- the sections in the rail's order, those that can be chosen now
    readonly property var order: SectionTable.sections.filter(s => !s.foot).concat(SectionTable.sections.filter(s => s.foot))
    function choosable(s) { return !DeviceInfoController.sender || s.sender === true }
    property var buttons: ({})
    // Shows the section `step` places from `from` (or the first or last, `edge`)
    // and gives its button the focus
    function move(from, step, edge) {
        const list = order.filter(s => choosable(s) || s.id === from)
        let at = list.findIndex(s => s.id === from)
        if (edge) at = step < 0 ? list.length : -1
        for (let i = 0; i < list.length; ++i) {
            at = (at + step + list.length) % list.length
            if (choosable(list[at])) break
        }
        const next = list[at]
        if (!next || next.id === from) return
        chosen(next.id)
        const button = buttons[next.id]
        if (button) button.forceActiveFocus(Qt.TabFocusReason)
    }
    // The section on show's button takes the focus (F6), with its ring
    function focusCurrent() {
        const button = buttons[current] || buttons[order.length ? order[0].id : ""]
        if (!button || !button.visible) return false
        button.forceActiveFocus(Qt.TabFocusReason)
        return button.activeFocus
    }

    implicitWidth: 76
    implicitHeight: 34
    color: Theme.sidebar
    Accessible.role: Accessible.PageTabList
    Accessible.name: qsTr("Sections")

    Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: Math.max(width, grid.implicitWidth)
        contentHeight: Math.max(height, grid.implicitHeight + grid.anchors.topMargin + grid.anchors.bottomMargin)
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height || contentWidth > width
        clip: true
        // -- WP LAY7 -- A section (or a device) below the edge of a low window comes
        // into sight with the keyboard, and the current one by itself
        ScrollBar.vertical: AppScrollBar { visible: flick.contentHeight > flick.height }
        function reveal(item) {
            const at = item.mapToItem(contentItem, 0, 0)
            if (at.y < contentY) contentY = Math.max(0, at.y)
            else if (at.y + item.height > contentY + height) contentY = Math.min(contentHeight - height, at.y + item.height)
            if (at.x < contentX) contentX = Math.max(0, at.x)
            else if (at.x + item.width > contentX + width) contentX = Math.min(contentWidth - width, at.x + item.width)
        }

        GridLayout {
            id: grid
            anchors.fill: parent
            anchors.leftMargin: rail.horizontal ? Theme.spacingSmall : 6
            anchors.rightMargin: rail.horizontal ? Theme.spacingSmall : 6
            anchors.topMargin: rail.horizontal ? 0 : 4
            anchors.bottomMargin: rail.horizontal ? 0 : 4
            columns: rail.horizontal ? SectionTable.sections.length + 2 : 1
            rowSpacing: 2
            columnSpacing: 2

            ColumnLayout {
                id: headerSlot
                visible: rail.headerShown && !rail.horizontal
                Layout.fillWidth: !rail.horizontal
                Layout.fillHeight: false
                spacing: 2
            }
            Repeater {
                model: SectionTable.sections.filter(s => !s.foot)
                delegate: RailButton {}
            }
            Item {
                Layout.fillWidth: rail.horizontal
                Layout.fillHeight: !rail.horizontal
            }
            Repeater {
                model: SectionTable.sections.filter(s => s.foot)
                delegate: RailButton {}
            }
        }
    }

    component RailButton: AbstractButton {
        id: button
        required property var modelData
        objectName: rail.objectName + "." + modelData.id
        checked: rail.current === modelData.id
        // An alink sender has no DSP: only what works on it (greyed, not hidden)
        enabled: !DeviceInfoController.sender || modelData.sender === true
        hoverEnabled: true
        // -- WP UA5 -- the rail is one Tab stop: the section on show (the first
        // while none is)
        focusPolicy: checked || (!rail.current.length && modelData === rail.order[0]) ? Qt.TabFocus : Qt.NoFocus
        Component.onCompleted: rail.buttons[modelData.id] = button
        Keys.onPressed: event => {
            const back = event.key === Qt.Key_Up || event.key === Qt.Key_Left
            const on = event.key === Qt.Key_Down || event.key === Qt.Key_Right
            if (event.modifiers !== Qt.NoModifier && event.modifiers !== Qt.KeypadModifier) return
            if (back || on) rail.move(modelData.id, back ? -1 : 1, false)
            else if (event.key === Qt.Key_Home || event.key === Qt.Key_End) rail.move(modelData.id, event.key === Qt.Key_Home ? 1 : -1, true)
            else return
            event.accepted = true
        }
        text: modelData.title
        Layout.fillWidth: !rail.horizontal
        implicitHeight: rail.horizontal ? 30 : 46
        implicitWidth: rail.horizontal ? flat.implicitWidth + 20 : 64
        readonly property color ink: !enabled ? Theme.textDisabled : checked ? Theme.text : Theme.textMuted

        contentItem: Item {
            Column {
                anchors.centerIn: parent
                visible: !rail.horizontal
                spacing: 3
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    name: button.modelData.icon
                    size: 18
                    color: button.ink
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(implicitWidth, button.width - 2)
                    text: button.text
                    font: Theme.fontCaption
                    color: button.ink
                    elide: Text.ElideRight
                }
            }
            Row {
                id: flat
                anchors.centerIn: parent
                visible: rail.horizontal
                spacing: 6
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    name: button.modelData.icon
                    size: 14
                    color: button.ink
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: button.text
                    font: Theme.fontBody
                    color: button.ink
                }
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            // -- WP DL5 -- the section on show in the selection tone (DESIGN-LANGUAGE §1)
            color: button.checked ? Theme.selection : button.hovered ? Theme.controlHover : "transparent"
            border.width: button.visualFocus ? Theme.focusRingWidth : 0
            border.color: Theme.focusRing
            // The mark of the section on show. -- WP DL5 -- in the rail, the
            // mockup's 3 × 16 accent bar at the item's left edge
            Rectangle {
                objectName: button.objectName + ".mark"
                visible: button.checked
                color: Theme.accent
                radius: rail.horizontal ? 1 : 1.5
                x: rail.horizontal ? 8 : -2
                y: rail.horizontal ? parent.height - 2 : (parent.height - height) / 2
                width: rail.horizontal ? parent.width - 16 : 3
                height: rail.horizontal ? 2 : 16
            }
        }
        onClicked: {
            // -- WP UA5 -- a click with the focus in the rail: the focus goes with
            // it, without a ring (no second marked item)
            const focused = rail.Window.activeFocusItem
            let inRail = false
            for (let i = focused; i; i = i.parent) if (i === rail) { inRail = true; break }
            rail.chosen(modelData.id)
            if (inRail && !button.activeFocus) button.forceActiveFocus(Qt.MouseFocusReason)
        }
        onActiveFocusChanged: if (activeFocus) flick.reveal(button)   // -- WP LAY7 --
        onCheckedChanged: if (checked) Qt.callLater(flick.reveal, button)
        // -- WP AIVIS -- an agent changed something in it, on show or not
        AiTouched { objectName: button.objectName + ".touched"; section: button.modelData.id }
        HelpTip {
            text: button.enabled ? "" : qsTr("This is an audio link sender. It has no DSP.")
            visible: button.hovered && text.length > 0
        }
        Accessible.role: Accessible.PageTab
        Accessible.name: text
        Accessible.selected: checked
        Accessible.onPressAction: button.clicked()
    }
}
