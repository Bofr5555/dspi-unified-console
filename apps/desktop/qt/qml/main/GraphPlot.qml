import QtQuick
import UConsole

// -- WP QP3 -- The response graph itself (the Mac's BodePlotView), in the main
// window and in the pop-out: a card with, bottom to top, the grid, the
// analyser's slot, the curves and the page's phase, the axis labels, the
// editor's slot, and the gear that opens the graph options while the pointer
// is over the plot.
//
// Overlay slots (filled by other packages, GraphController::Slot):
// - GraphController.RtaOverlay (QP9): the spectrum, above the grid and under
//   the curves. Its item fills the plot and gets `plot` (this item) when it
//   declares the property.
// - GraphController.PeqOverlay (QP4): the on-graph editor, above everything but
//   the gear; same `plot`. It sets GraphController.overlayChannel to the
//   channel whose curve it draws itself, which the curves here then leave out
//   (in the main window and in a pop-out that follows the page).
// What `plot` offers a slot's item:
// - the mapping, from the curve layer: xForFrequency(hz), frequencyForX(x),
//   yForDb(db), dbForY(y), dbStep();
// - popOut, following, onDashboard, activeChannel, channels (wire channels
//   whose curves show) and channelColors (a colour per wire channel);
// - the scale itself: GraphController.minFreq, maxFreq, dbTop, dbBottom.
Rectangle {
    id: plot
    // The pop-out follows the page unless switched off in its setup page
    readonly property bool following: !popOut || GraphController.popoutFollowsSelection
    readonly property var channels: popOut ? GraphController.popOutVisibleChannels : GraphController.visibleChannels
    readonly property int activeChannel: GraphController.selectedChannel
    readonly property bool onDashboard: GraphController.selectedChannel < 0
    readonly property var channelColors: colorsOf(DeviceModel.revision)
    readonly property alias optionsOpen: options.open
    // In the pop-out window (its own visibility, its own names)
    property bool popOut: false
    // -- WP QPI2 -- the PEQ slot's editor (a PeqOverlay), for the band bar
    // docked under this graph; null without one
    readonly property var peqEditor: peqSlot.item && peqSlot.item.editor ? peqSlot.item.editor : null

    objectName: popOut ? "GraphWindow.plot" : "GraphHost.plot"
    color: Theme.panel
    radius: Theme.radius
    border { color: Theme.separator; width: 1 }
    clip: true

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("Filter Response")

    // A colour per wire channel, ChannelPalette's for its sidebar row
    function colorsOf(revision) {
        const rows = DeviceModel.channels
        const out = []
        for (let r = 0; r < rows.count; ++r)
            out[rows.get(r, "channel")] = ChannelPalette.forRow(rows.get(r, "kind"), rows.get(r, "number"), rows.get(r, "pdm"))
        return out
    }

    function xForFrequency(hz) { return curves.xForFrequency(hz) }
    function frequencyForX(x) { return curves.frequencyForX(x) }
    function yForDb(db) { return curves.yForDb(db) }
    function dbForY(y) { return curves.dbForY(y) }
    function dbStep() { return curves.dbStep() }

    component SlotLoader: Loader {
        required property int kind
        readonly property string type: GraphController.slotType(kind)
        anchors.fill: parent
        active: type !== ""
        sourceComponent: active ? Qt.createComponent("UConsole", type) : null
        onLoaded: if ("plot" in item) item.plot = plot
    }

    BodePlot {
        objectName: plot.objectName + ".grid"
        anchors.fill: parent
        graph: GraphController
        layers: BodePlot.Grid
        gridColor: Theme.text
    }

    SlotLoader {
        objectName: plot.objectName + ".rtaSlot"
        kind: GraphController.RtaOverlay
    }

    BodePlot {
        id: curves
        objectName: plot.objectName + ".curves"
        anchors.fill: parent
        graph: GraphController
        layers: BodePlot.Curves
        channels: plot.channels
        activeChannel: plot.activeChannel
        followsSelection: plot.following
        excludeOverlay: plot.following
        channelColors: plot.channelColors
        phaseColor: Theme.text
        labelColor: Theme.textMuted
        // Curves fade with the device; the grid underneath stays
        opacity: DeviceModel.connected ? 1.0 : 0.0
        Behavior on opacity { OpacityAnimator { duration: 300 } }
    }

    BodePlot {
        objectName: plot.objectName + ".labels"
        anchors.fill: parent
        graph: GraphController
        layers: BodePlot.Labels
        labelColor: Theme.textMuted
        phaseColor: Theme.text
        labelFont: Theme.fontCaption
    }

    SlotLoader {
        id: peqSlot
        objectName: plot.objectName + ".peqSlot"
        kind: GraphController.PeqOverlay
    }

    HoverHandler {
        id: hover
    }

    GraphOptions {
        id: options
        popOut: plot.popOut
        anchors { right: parent.right; top: parent.top; margins: 6 }
        // On hover; an open popover keeps it, or it would vanish from under
        // it; and while it has the keyboard focus
        opacity: hover.hovered || open || activeFocus ? 1.0 : 0.0
        Behavior on opacity { OpacityAnimator { duration: 150 } }
    }
}
