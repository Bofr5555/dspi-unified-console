import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- Library > Presets: every preset slot of the device, with what can
// be done to it (the slot's menu, by its button or a right-click: ContextMenus)
// and Recall. The active one says whether it has unsaved changes.
Item {
    id: page
    objectName: "LibraryPresets"
    property bool shown: false
    property Item host: null

    readonly property bool writable: DeviceModel.connected && !DeviceModel.readOnly && !SidebarController.presetBusy
    function menu(slot, item, x, y) { ContextMenus.slot(slot, page.host ? page.host.dialogs : null, item, x, y) }

    Text {
        objectName: "LibraryPresets.none"
        anchors.centerIn: parent
        visible: !DeviceModel.connected
        text: qsTr("No Device Connected")
        font: Theme.fontTitle
        color: Theme.textMuted
    }
    ListView {
        id: list
        objectName: "LibraryPresets.list"
        visible: DeviceModel.connected
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(760, parent.width - 2 * Theme.spacingLarge)
        height: parent.height
        topMargin: Theme.spacingLarge
        bottomMargin: Theme.spacingLarge
        spacing: 2
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: DeviceModel.presets
        ScrollBar.vertical: AppScrollBar {}
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Presets")

        header: ColumnLayout {
            width: list.width
            spacing: Theme.spacingSmall
            Text {
                objectName: "LibraryPresets.startup"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: DeviceModel.presets.startupMode === 0
                      //: %1 is a slot's number
                      ? qsTr("At power-on the device loads preset %1.").arg(DeviceModel.presets.defaultSlot + 1)
                      : qsTr("At power-on the device loads the preset it was last on.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
                bottomPadding: Theme.spacingSmall
            }
        }
        delegate: Rectangle {
            id: row
            required property int slot
            required property string name
            required property bool occupied
            required property bool active
            readonly property bool startup: DeviceModel.presets.startupMode === 0 && DeviceModel.presets.defaultSlot === slot
            readonly property string title: name.length ? name : occupied || active ? qsTr("Preset %1").arg(slot + 1) : qsTr("Empty")
            readonly property string note: [active ? (SidebarController.unsaved ? qsTr("Active, with unsaved changes") : qsTr("Active")) : "",
                                            startup ? qsTr("Startup") : ""].filter(t => t.length).join(" · ")
            objectName: "LibraryPresets.slot." + slot
            width: ListView.view.width
            implicitHeight: 44
            radius: Theme.radiusSmall
            color: active ? Theme.selection : hover.hovered ? Theme.controlHover : Theme.panel
            HoverHandler { id: hover }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: point => page.menu(row.slot, row, point.position.x, point.position.y)
            }
            Accessible.role: Accessible.ListItem
            Accessible.name: (slot + 1) + ": " + title + (note.length ? ", " + note : "")

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.spacing
                anchors.rightMargin: Theme.spacingSmall
                spacing: Theme.spacing
                Text {
                    Layout.preferredWidth: 22
                    horizontalAlignment: Text.AlignRight
                    text: row.slot + 1
                    font: Theme.fontMono
                    color: Theme.textMuted
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Text {
                        objectName: row.objectName + ".name"
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: row.title
                        font: Theme.fontBody
                        color: row.occupied || row.active ? Theme.text : Theme.textMuted
                        elide: Text.ElideRight
                    }
                    Text {
                        objectName: row.objectName + ".note"
                        Layout.fillWidth: true
                        visible: text.length > 0
                        text: row.note
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                }
                AppButton {
                    objectName: row.objectName + ".recall"
                    visible: !row.active
                    enabled: page.writable
                    text: qsTr("Recall")
                    Accessible.name: qsTr("Recall") + " " + (row.slot + 1) + ": " + row.title
                    onClicked: SidebarController.requestPresetSwitch(row.slot)
                }
                AppButton {
                    objectName: row.objectName + ".save"
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
                    iconName: "chevron-down"
                    toolTip: qsTr("More")
                    Accessible.name: qsTr("More") + ", " + (row.slot + 1) + ": " + row.title
                    onClicked: page.menu(row.slot, more, 0, more.height + 2)
                }
            }
        }
    }
}
