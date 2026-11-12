import QtQuick
import UConsole

// -- WP QP9 -- The live spectrum on the response graph (the Mac's
// GraphSpectrumOverlay), in GraphController's RtaOverlay slot: above the grid,
// under the curves, in each channel's curve colour. It shows on the page's
// "FFT Graph" switch, draws the device's selection while following another
// console, and asks for the analyser only while it is on screen. It takes no
// pointer input at all (no hover of its own: the graph's HoverHandler and the
// gear keep theirs).
Item {
    id: overlay
    // GraphPlot sets it (the slot contract)
    property var plot: null
    readonly property bool popOut: plot ? plot.popOut : false
    readonly property var win: Window.window
    readonly property bool onScreen: plot !== null && visible && win !== null && win.visible
                                     && (popOut ? GraphController.poppedOut : !GraphController.poppedOut)
    readonly property bool active: onScreen && RtaController.supported && RtaController.showGraph
                                   && RtaController.ownChannels.length > 0 || (onScreen && RtaController.supported
                                   && RtaController.showGraph && RtaController.following)
    readonly property string viewId: popOut ? "graph.popOut" : "graph"

    objectName: popOut ? "GraphWindow.spectrum" : "GraphHost.spectrum"
    enabled: false
    opacity: active ? 1 : 0
    Behavior on opacity { OpacityAnimator { duration: 200 } }

    onActiveChanged: RtaController.setView(viewId, RtaController.Curves, active)
    Component.onCompleted: RtaController.setView(viewId, RtaController.Curves, active)
    Component.onDestruction: RtaController.setView(viewId, RtaController.Curves, false)

    function colorsFor(tap, channels, revision) {
        const out = []
        for (const ch of channels) {
            const info = RtaController.channelInfo(tap, ch)
            out.push(ChannelPalette.forRow(info.kind, info.number, info.pdm))
        }
        return out
    }

    RtaSurface {
        id: surface
        objectName: overlay.objectName + ".surface"
        anchors.fill: parent
        active: overlay.active
        tap: RtaController.drawnTap
        channels: RtaController.drawnChannels
        colors: overlay.colorsFor(RtaController.drawnTap, RtaController.drawnChannels, DeviceModel.revision)
        minFreq: GraphController.minFreq
        maxFreq: GraphController.maxFreq
        glow: GraphController.showGraphGlow
    }
}
