import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- Library > Presets: every preset slot of the device, with what can
// be done to it (the slot's menu, by its button or a right-click: ContextMenus)
// and Recall. The active one says whether it has unsaved changes.
// -- WP DL3a -- The slots as ListRows in the 760 px column at the page's left
// (DESIGN-LANGUAGE §3, §9): number, name, a caption with the state; Recall,
// Save and the menu show on hover and on the active row; a double click
// recalls. Save is the one accent button, while there is something to save.
Item {
    id: page
    objectName: "LibraryPresets"
    property bool shown: false
    property Item host: null

    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly && !SidebarController.presetBusy
    function menu(slot, item, x, y) { ContextMenus.slot(slot, page.host ? page.host.dialogs : null, item, x, y) }

    EmptyState {
        objectName: "LibraryPresets.none"
        anchors.fill: parent
        visible: !DeviceModel.connected
        iconName: "unplug"
        title: qsTr("No Device Connected")
        text: qsTr("Presets live on the device. Open one to see its slots.")
    }
    ListView {
        id: list
        objectName: "LibraryPresets.list"
        visible: DeviceModel.connected
        x: Theme.spacing
        width: Math.min(Theme.contentMaxWidth, parent.width - 2 * Theme.spacing)
        height: parent.height
        topMargin: Theme.spacing
        bottomMargin: Theme.spacing
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        activeFocusOnTab: true
        keyNavigationEnabled: true
        currentIndex: -1
        model: DeviceModel.presets
        ScrollBar.vertical: AppScrollBar {}
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Presets")

        header: Text {
            objectName: "LibraryPresets.startup"
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
            bottomPadding: Theme.spacing
        }
        delegate: ListRow {
            id: row
            required property int index
            required property int slot
            required property string name
            required property bool occupied
            required property bool active
            readonly property bool startup: DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === slot
            readonly property string label: name.length ? name : occupied || active ? qsTr("Preset %1").arg(slot + 1) : qsTr("Empty")
            readonly property string note: [active ? (SidebarController.unsaved ? qsTr("Active, with unsaved changes") : qsTr("Active")) : "",
                                            startup ? qsTr("Startup") : ""].filter(t => t.length).join(" · ")
            objectName: "LibraryPresets.slot." + slot
            width: ListView.view.width
            rowIndex: index
            title: label
            subtitle: note
            selected: active
            iconName: startup ? "power" : ""
            // The actions show on hover and on the active row; they stay
            // enabled for the keyboard and for a click that comes first
            actionsAlways: true
            readonly property real actionOpacity: hovered || selected || current || activeFocus ? 1 : 0
            onClicked: list.currentIndex = index
            onActivated: if (!active && page.writable) SidebarController.requestPresetSwitch(slot)
            onMenuRequested: (x, y) => page.menu(row.slot, row, x, y)
            leading: Text {
                Layout.preferredWidth: 22
                horizontalAlignment: Text.AlignRight
                text: row.slot + 1
                font: Theme.fontMono
                color: Theme.textMuted
            }
            Accessible.name: (slot + 1) + ": " + label + (note.length ? ", " + note : "")

            AppButton {
                objectName: row.objectName + ".recall"
                opacity: row.actionOpacity || activeFocus ? 1 : 0
                visible: !row.active
                enabled: page.writable
                text: qsTr("Recall")
                Accessible.name: qsTr("Recall") + " " + (row.slot + 1) + ": " + row.label
                onClicked: SidebarController.requestPresetSwitch(row.slot)
            }
            AppButton {
                objectName: row.objectName + ".save"
                opacity: row.actionOpacity || activeFocus ? 1 : 0
                visible: row.active
                enabled: page.writable
                primary: SidebarController.unsaved
                text: qsTr("Save")
                toolTip: qsTr("Save the current settings in this slot")
                onClicked: SidebarController.savePreset()
            }
            IconButton {
                id: more
                objectName: row.objectName + ".menu"
                opacity: row.actionOpacity || activeFocus ? 1 : 0
                iconName: "chevron-down"
                toolTip: qsTr("More")
                Accessible.name: qsTr("More") + ", " + (row.slot + 1) + ": " + row.label
                onClicked: page.menu(row.slot, more, 0, more.height + 2)
            }
        }
    }
}
