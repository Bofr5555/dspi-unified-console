import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- One row of a list or simple table (DESIGN-LANGUAGE §9): the
// shared delegate for Presets, Snapshots, Events, Documents, devices and the
// AutoEQ list. Theme.rowHeight high (28 in Modern; more with a subtitle),
// Theme.stripe on every second row (`rowIndex` odd), the hover tone under
// the pointer, the selection tone when `selected`. From the left: `leading`
// (a channel pill, a status dot, a badge), an icon, the title in fontTable
// with an optional subtitle in fontSmall muted, the value at the right (mono
// when `monoValue`), then the row actions: quiet buttons (IconButton) that
// show while the row is hovered, selected or current, or one of them has the
// focus, or always with `actionsAlways`. Hidden they fade out but stay
// enabled (a click, the keyboard and a script still reach them).
//
// The list owns the keyboard: as a ListView delegate the row draws the focus
// ring while it is the current item and the view has the focus. A click emits
// `clicked(modifiers)`, a double click `activated()`, a right click (or the
// menu key while current) `menuRequested(x, y)` in row coordinates.
//
//     ListView {
//         id: list
//         model: presets
//         keyNavigationEnabled: true
//         delegate: ListRow {
//             required property int index
//             required property string name
//             objectName: "Presets.row." + index   // actions get their own names
//             width: ListView.view.width
//             rowIndex: index
//             title: name
//             subtitle: qsTr("Saved 12 May")      // optional
//             value: "−3.0 dB"                    // optional
//             selected: list.currentIndex === index
//             onClicked: list.currentIndex = index
//             onActivated: presets.recall(index)
//             IconButton { iconName: "trash"; toolTip: qsTr("Delete"); onClicked: presets.remove(index) }
//         }
//     }
Item {
    id: row
    property int rowIndex: -1
    property string title
    // -- INTC (DL3a fold) -- the title quiet, as a placeholder (an empty preset slot's "Empty")
    property bool titleMuted: false
    property string subtitle
    property string value
    property bool monoValue: true
    property string iconName
    property color iconColor: Theme.textMuted
    property bool selected: false
    property bool actionsAlways: false
    property alias leading: leadingSlot.data
    default property alias actions: actionRow.data
    readonly property bool hovered: hover.hovered
    // The keyboard is on this row: the current item of a focused ListView
    readonly property bool current: row.ListView.isCurrentItem && row.ListView.view !== null && row.ListView.view.activeFocus
    signal clicked(int modifiers)
    signal activated()
    signal menuRequested(real x, real y)

    implicitHeight: subtitle.length > 0 ? Theme.rowHeight + 14 : Theme.rowHeight
    implicitWidth: line.implicitWidth + 2 * Theme.spacingSmall

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusSmall
        color: row.selected ? Theme.selection
             : row.hovered && row.enabled ? Theme.controlHover
             : row.rowIndex % 2 === 1 ? Theme.stripe : "transparent"
        border.width: row.current ? Theme.focusRingWidth : 0
        border.color: Theme.focusRing
    }

    HoverHandler { id: hover }
    TapHandler {
        acceptedButtons: Qt.LeftButton
        onTapped: (point, button) => row.clicked(point.modifiers)
        onDoubleTapped: row.activated()
    }
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: point => row.menuRequested(point.position.x, point.position.y)
    }
    Keys.onMenuPressed: menuRequested(Theme.spacingSmall, height)
    Keys.onReturnPressed: activated()
    Keys.onEnterPressed: activated()

    RowLayout {
        id: line
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingSmall
        anchors.rightMargin: Theme.spacingSmall / 2
        spacing: Theme.spacingSmall

        RowLayout {
            id: leadingSlot
            // -- WP DL0 fold -- as wide as what it holds: a slot whose items
            // are all hidden takes no room from the title
            Layout.fillWidth: false
            spacing: 6
            visible: children.length > 0
        }
        Icon {
            visible: row.iconName.length > 0
            name: row.iconName
            color: row.enabled ? row.iconColor : Theme.textDisabled
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            spacing: 0
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: row.title
                font: Theme.fontTable
                color: !row.enabled ? Theme.textDisabled : row.titleMuted ? Theme.textMuted : Theme.text
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                visible: row.subtitle.length > 0
                textFormat: Text.PlainText
                text: row.subtitle
                font: Theme.fontSmall
                color: row.enabled ? Theme.textMuted : Theme.textDisabled
                elide: Text.ElideRight
            }
        }
        Text {
            visible: row.value.length > 0
            textFormat: Text.PlainText
            text: row.value
            font: row.monoValue ? Theme.fontMono : Theme.fontTable
            color: row.enabled ? Theme.text : Theme.textDisabled
            horizontalAlignment: Text.AlignRight
        }
        RowLayout {
            id: actionRow
            spacing: 2
            visible: children.length > 0
            // -- WP DL0 fold -- hidden actions fade out but stay enabled: a
            // click that comes before the hover, the keyboard and a scripted
            // press still reach them, and one with the focus shows
            readonly property bool focused: {
                for (let i = 0; i < children.length; ++i)
                    if (children[i].activeFocus) return true
                return false
            }
            readonly property bool shown: row.actionsAlways || row.hovered || row.selected || row.current || focused
            opacity: shown ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 120 } }
        }
    }

    Accessible.role: Accessible.ListItem
    Accessible.name: title
    Accessible.description: subtitle.length > 0 ? subtitle + (value.length ? ", " + value : "") : value
    Accessible.selected: selected
    Accessible.selectable: true
}
