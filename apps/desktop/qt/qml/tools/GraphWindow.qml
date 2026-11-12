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
            popOut: true
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.spacingLarge
            Layout.bottomMargin: Theme.spacingLarge
        }
        GraphLegend {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.bottomMargin: Theme.spacingSmall
        }
    }
}
