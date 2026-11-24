import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP DL5 -- The top bar's preset control as one popover (DESREV §5, owner
// approved; DESIGN-LANGUAGE §9, §15), in place of the combo box's list and the
// slot menu beside it: every slot of the device as a ListRow (number, name,
// what it is: active, unsaved, startup), with its actions on hover, on the
// active row and on the keyboard's row: Recall, Save on the active slot, Copy
// Here on an empty one, and the slot's menu (ContextMenus.slot: Rename, Set as
// Startup, Copy to, Clear). A double click or Enter recalls. The list is the
// popover's contentItem; its rows are itemAtIndex(slot).
//
//     PresetPopover {
//         parent: button
//         y: button.height + 4
//         dialogs: shell's PresetDialogs (the slot menu's questions)
//     }
PopoverPanel {
    id: popover
    objectName: "PresetPopover"
    title: qsTr("Presets")
    // ModernShell's PresetDialogs: what the slot menu asks
    property Item dialogs: null
    // How high it may grow (the room under the top bar)
    property real maximumHeight: 600

    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly && !SidebarController.presetBusy
    readonly property int activeSlot: DeviceModel.presets.activeSlot

    // The slot's menu where `item` is, the popover making way for it
    function menu(slot, item, x, y) {
        ContextMenus.slot(slot, dialogs, item, x, y)
        close()
    }
    function recall(slot) {
        close()
        SidebarController.requestPresetSwitch(slot)
    }

    width: 360
    height: Math.min(implicitHeight, maximumHeight)
    padding: 6

    // Opened from the keyboard: its row starts on the active slot (with the
    // focus ring); from the pointer: no row until an arrow key
    property bool fromKeyboard: false

    onAboutToShow: list.currentIndex = fromKeyboard ? activeSlot : -1
    onOpened: list.forceActiveFocus()

    // A floating surface (DESIGN-LANGUAGE §5): panel tone, radius 8, a shadow,
    // and a 1 px edge in the dark scheme only
    background: Item {
        Repeater {
            model: 4
            Rectangle {
                required property int index
                readonly property real spread: Theme.shadowBlur * (index + 1) / 8
                x: -spread
                y: Theme.shadowOffset * (index + 1) / 4 - spread
                width: parent.width + 2 * spread
                height: parent.height + 2 * spread
                radius: Theme.radius + spread
                color: Qt.rgba(Theme.shadowColor.r, Theme.shadowColor.g, Theme.shadowColor.b, Theme.shadowColor.a / 6)
            }
        }
        Rectangle {
            anchors.fill: parent
            radius: Theme.radius
            color: Theme.panel
            border.width: Theme.dark ? 1 : 0
            border.color: Theme.separator
        }
        Accessible.role: Accessible.Dialog
        Accessible.name: popover.title
    }

    contentItem: ListView {
        id: list
        objectName: "PresetPopover.list"
        implicitWidth: 360 - 2 * popover.padding
        implicitHeight: contentHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        keyNavigationEnabled: true
        currentIndex: -1
        model: DeviceModel.presets
        ScrollBar.vertical: AppScrollBar {}
        Accessible.role: Accessible.List
        Accessible.name: popover.title

        delegate: ListRow {
            id: row
            required property int index
            required property int slot
            required property string name
            required property bool occupied
            required property bool active
            readonly property bool empty: !occupied && !active
            readonly property bool startup: DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === slot
            readonly property string label: name.length ? name : empty ? "" : qsTr("Preset %1").arg(slot + 1)
            readonly property string note: [active ? (SidebarController.unsaved ? qsTr("Active, with unsaved changes") : qsTr("Active")) : "",
                                            startup ? qsTr("Startup") : ""].filter(t => t.length).join(" · ")
            objectName: "PresetPopover.slot." + slot
            width: ListView.view.width
            rowIndex: index
            title: label
            subtitle: note
            selected: active
            iconName: startup ? "power" : ""
            onClicked: list.currentIndex = index
            onActivated: if (!active && occupied && popover.writable) popover.recall(slot)
            onMenuRequested: (x, y) => popover.menu(row.slot, row, x, y)
            leading: [
                Text {
                    Layout.preferredWidth: 18
                    horizontalAlignment: Text.AlignRight
                    text: row.slot + 1
                    font: Theme.fontMono
                    color: Theme.textMuted
                },
                // An empty slot says so quietly, not as a name
                Text {
                    visible: row.empty
                    text: qsTr("Empty")
                    font: Theme.fontTable
                    color: Theme.textMuted
                }
            ]
            Accessible.name: (slot + 1) + ": " + (empty ? qsTr("Empty") : label) + (note.length ? ", " + note : "")

            AppButton {
                objectName: row.objectName + ".recall"
                visible: !row.active && row.occupied
                implicitHeight: 22
                enabled: popover.writable
                text: qsTr("Recall")
                Accessible.name: qsTr("Recall") + " " + (row.slot + 1) + ": " + row.label
                onClicked: popover.recall(row.slot)
            }
            AppButton {
                objectName: row.objectName + ".copy"
                visible: row.empty
                implicitHeight: 22
                enabled: popover.writable && popover.activeSlot >= 0
                text: qsTr("Copy Here")
                //: %1 is the active slot's label, "3: Living Room"
                toolTip: qsTr("Copy \"%1\" into this slot").arg(SidebarController.presetLabel(popover.activeSlot))
                onClicked: SidebarController.requestPresetCopy(row.slot)
            }
            AppButton {
                objectName: row.objectName + ".save"
                visible: row.active
                implicitHeight: 22
                enabled: popover.writable
                primary: SidebarController.unsaved
                text: qsTr("Save")
                toolTip: qsTr("Save the current settings in this slot")
                onClicked: SidebarController.savePreset()
            }
            IconButton {
                id: more
                objectName: row.objectName + ".menu"
                iconName: "chevron-down"
                toolTip: qsTr("More")
                Accessible.name: qsTr("More") + ", " + (row.slot + 1) + ": " + (row.empty ? qsTr("Empty") : row.label)
                onClicked: popover.menu(row.slot, more, 0, more.height + 2)
            }
        }

        footer: Text {
            objectName: "PresetPopover.startup"
            width: list.width
            textFormat: Text.PlainText
            text: DeviceModel.presets.startupMode === 0
                  //: %1 is a slot's number
                  ? qsTr("At power-on the device loads preset %1.").arg(DeviceModel.presets.defaultSlot + 1)
                  : qsTr("At power-on the device loads the preset it was last on.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
            leftPadding: Theme.spacingSmall
            rightPadding: Theme.spacingSmall
            topPadding: Theme.spacingSmall
            bottomPadding: 2
        }
    }
}
