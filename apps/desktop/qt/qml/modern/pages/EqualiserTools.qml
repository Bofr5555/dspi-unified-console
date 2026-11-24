import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY4 -- The Equaliser's buttons in the row over its pages (MODLAY 3.2):
// the headphone profiles, the filter files, and the inspector.
// -- WP DL8 -- Headphone Profile only while an input is selected (a profile
// is an input's), and Import / Export under one "Files ▾" (DESREV §5).
Row {
    id: tools
    objectName: "EqualiserTools"
    spacing: 6

    ChannelInfo { id: selected; channel: SidebarController.selectedChannel }

    component Tool: AppButton {
        required property string actionId
        objectName: "EqualiserTools." + actionId
        enabled: Shell.allows(actionId)
        toolTip: MenuTable.shortcutOf(actionId)
        onClicked: Shell.trigger(actionId)
    }
    component FileRow: AppMenuItem {
        required property string actionId
        objectName: "EqualiserTools.files.menu." + actionId
        text: MenuTable.row(actionId).text
        shortcutText: MenuTable.shortcutOf(actionId)
        enabled: Shell.allows(actionId)
        onTriggered: Shell.trigger(actionId)
    }
    Tool {
        actionId: "autoeq.browse"
        visible: selected.known && !selected.isOutput
        text: qsTr("Headphone Profile…")
    }
    AppButton {
        id: files
        objectName: "EqualiserTools.files"
        //: A button that opens a menu: Import Filters, Export Filters
        text: qsTr("Files") + " ▾"
        enabled: Shell.allows("file.importFilters") || Shell.allows("file.exportFilters")
        onClicked: menu.popup(files, files.width - menu.width, files.height + 2)
        Accessible.role: Accessible.ButtonMenu
        Accessible.name: qsTr("Files")

        AppMenu {
            id: menu
            objectName: "EqualiserTools.files.menu"
            FileRow { actionId: "file.importFilters" }
            FileRow { actionId: "file.exportFilters" }
        }
    }
    // -- WP LAY5a -- a type of its own, for the other pages
    InspectorButton { objectName: "EqualiserTools.inspector" }
}
