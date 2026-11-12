import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The pop-out "Filter Response" window (the Mac's
// GraphWindowController and GraphPopOutView): the graph with its gear, and the
// legend under it. 800×400, at least 500×250. While it is open the main
// window's graph steps aside; it follows the selected page unless its setup
// page says otherwise. Opened by Window > Pop Out Graph and the gear's row.
ToolWindow {
    id: window
    objectName: "GraphWindow"
    windowId: "graph"
    defaultWidth: 800
    defaultHeight: 400
    minimumWidth: 500
    minimumHeight: 250
    title: qsTr("Filter Response")

    onVisibleChanged: GraphController.poppedOut = visible
    Component.onCompleted: GraphController.poppedOut = visible

    Connections {
        target: GraphController
        function onPopOutCloseRequested() { window.close() }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        GraphPlot {
            id: plot
            popOut: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.spacingLarge
            Layout.bottomMargin: dock.item ? 0 : Theme.spacingLarge
        }
        // -- WP QPI2 -- the band bar under the pop-out's graph too
        Loader {
            id: dock
            objectName: "GraphWindow.peqDock"
            readonly property string type: GraphController.slotType(GraphController.PeqDock)
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: item ? 6 : 0
            Layout.bottomMargin: item ? Theme.spacingLarge : 0
            active: type !== ""
            sourceComponent: active ? Qt.createComponent("UConsole", type) : null
            onLoaded: if ("plot" in item) item.plot = plot
        }
        GraphLegend {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.bottomMargin: Theme.spacingSmall
        }
    }
}
