import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- The Modern layout's inspector: what is selected, at the page's
// right. In a row of a layout it takes its width while it is beside the page
// and none while it is hidden or over it (InspectorState). Its content is made
// while it can be seen only, so a limiter's poll and a draft end with it.
Item {
    id: inspector
    // The page shows
    property bool shown: false
    property Component content: null
    readonly property bool open: shown && InspectorState.open
    readonly property Item body: loader.item

    objectName: "Inspector"
    implicitWidth: open && !InspectorState.narrow ? InspectorState.width : 0
    // Over the page, when it is not beside it
    z: 2

    Rectangle {
        id: sheet
        objectName: inspector.objectName + ".sheet"
        visible: inspector.open
        x: inspector.width - width
        width: InspectorState.width
        height: parent.height
        color: InspectorState.narrow ? Theme.menu : Theme.sidebar
        // Its clicks and its wheel are its own
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: wheel => wheel.accepted = true
        }
        Rectangle {
            visible: InspectorState.narrow
            width: 1
            height: parent.height
            color: Theme.border
        }
        IconButton {
            objectName: inspector.objectName + ".close"
            z: 1
            anchors { top: parent.top; right: parent.right; margins: 8 }
            iconName: "x"
            iconSize: 12
            iconColor: Theme.textMuted
            toolTip: qsTr("Hide the Inspector")
            onClicked: InspectorState.toggle()
        }
        Flickable {
            id: flick
            objectName: inspector.objectName + ".scroll"
            anchors.fill: parent
            contentWidth: width
            contentHeight: loader.implicitHeight + 2 * Theme.spacing
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            ScrollBar.vertical: AppScrollBar {}
            Loader {
                id: loader
                x: Theme.spacing
                y: Theme.spacing
                width: flick.width - 2 * Theme.spacing - 6
                active: inspector.open && DeviceModel.connected
                sourceComponent: inspector.content
            }
        }
    }

    // A field the keyboard moved to is in sight
    readonly property Item focusItem: open ? Window.activeFocusItem : null
    onFocusItemChanged: {
        if (!focusItem || !loader.item) return
        let up = focusItem
        while (up && up !== loader) up = up.parent
        if (!up) return
        const top = focusItem.mapToItem(loader, 0, 0).y
        if (top < flick.contentY) flick.contentY = Math.max(0, top - Theme.spacing)
        else if (top + focusItem.height + 2 * Theme.spacing > flick.contentY + flick.height)
            flick.contentY = Math.max(0, Math.min(flick.contentHeight - flick.height, top + focusItem.height + 2 * Theme.spacing - flick.height))
    }

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Inspector")
}
