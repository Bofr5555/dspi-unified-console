import QtQuick
import UConsole

// -- WP LAY4 -- The Equaliser's buttons in the row over its pages (MODLAY 3.2):
// the headphone profiles, the filter files, and the inspector.
Row {
    id: tools
    objectName: "EqualiserTools"
    spacing: 6

    component Tool: AppButton {
        required property string actionId
        objectName: "EqualiserTools." + actionId
        enabled: Shell.allows(actionId)
        toolTip: MenuTable.shortcutOf(actionId)
        onClicked: Shell.trigger(actionId)
    }
    Tool { actionId: "autoeq.browse"; text: qsTr("Headphone Profile…") }
    Tool { actionId: "file.importFilters"; text: qsTr("Import…") }
    Tool { actionId: "file.exportFilters"; text: qsTr("Export…") }
    // -- WP LAY5a -- a type of its own, for the other pages
    InspectorButton { objectName: "EqualiserTools.inspector" }
}
