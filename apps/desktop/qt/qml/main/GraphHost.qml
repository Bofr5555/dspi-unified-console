import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph at the top of the right pane (the Mac's ContentView
// header, BodePlotView and GraphResizeHandle): the graph at the height the
// user dragged it to, and the drag strip under it. While the graph is popped
// out a header holds a button that brings it back and the graph is not drawn
// here. No legend: the sidebar's curve pills are the main
// window's show/hide control.
//
// The PEQ editor's band bar docks under the graph (GraphController.PeqDock,
// PeqBandBar): its item gets `plot` (GraphPlot) when it declares the property.
// The right pane supplies the side margins.
ColumnLayout {
    id: host
    objectName: "GraphHost"
    // The right pane's selection: -1 for the overview, else a wire channel
    property int selectedChannel: -1
    // The other half of a linked input pair shown with it, or -1 (QP2)
    property int linkedPartner: -1

    spacing: 0
    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Filter Response")
    onSelectedChannelChanged: GraphController.select(selectedChannel, linkedPartner)
    onLinkedPartnerChanged: GraphController.select(selectedChannel, linkedPartner)
    Component.onCompleted: GraphController.select(selectedChannel, linkedPartner)

    // ---- Header, only while popped out: the button that brings the graph back ----
    // -- WP QTTWEAK -- no "Filter Response" heading over the graph (its space
    // went to the graph); the graph keeps the name for assistive technology
    Item {
        visible: GraphController.poppedOut
        implicitHeight: 22
        Layout.fillWidth: true
        // Popped out, no graph or strip follows, so the header carries the gap
        Layout.bottomMargin: 14

        IconButton {
            objectName: "GraphHost.closePopOut"
            anchors { left: parent.left; verticalCenter: parent.verticalCenter }
            iconName: "minimize-2"
            iconSize: 12
            iconColor: Theme.textMuted
            toolTip: qsTr("Close pop-out window")
            onClicked: GraphController.closePopOut()
        }
    }

    // ---- The graph ----
    GraphPlot {
        id: plot
        visible: !GraphController.poppedOut
        Layout.fillWidth: true
        Layout.preferredHeight: GraphController.graphHeight
    }

    Loader {
        objectName: "GraphHost.peqDock"
        readonly property string type: GraphController.slotType(GraphController.PeqDock)
        visible: !GraphController.poppedOut
        Layout.fillWidth: true
        Layout.topMargin: item ? 6 : 0 // -- WP QPI2 --
        active: type !== ""
        sourceComponent: active ? Qt.createComponent("UConsole", type) : null
        onLoaded: if ("plot" in item) item.plot = plot
    }

    // ---- The drag strip: sets the graph's height ----
    Item {
        id: strip
        objectName: "GraphHost.resize"
        visible: !GraphController.poppedOut
        Layout.fillWidth: true
        implicitHeight: 12
        Accessible.role: Accessible.Separator
        Accessible.name: qsTr("Graph height")

        property real startHeight: 0
        Rectangle {
            anchors.centerIn: parent
            width: 36
            height: 4
            radius: 2
            color: Theme.border
            opacity: dragHover.hovered || drag.active ? 1 : 0.5
        }
        HoverHandler {
            id: dragHover
            cursorShape: Qt.SizeVerCursor
        }
        DragHandler {
            id: drag
            target: null
            yAxis.enabled: true
            xAxis.enabled: false
            cursorShape: Qt.SizeVerCursor
            onActiveChanged: if (active) strip.startHeight = GraphController.graphHeight
            onTranslationChanged: if (active) GraphController.graphHeight = strip.startHeight + translation.y
        }
    }
}
