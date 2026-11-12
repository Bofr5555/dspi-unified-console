pragma Singleton
import QtQuick
import UConsole

// -- WP QP4 -- The on-graph editor's side of the band list: FilterList's
// `graphSelection` (see FilterList.qml), over PeqEditorController, and the
// bands' colours (the Mac's PeqBandPalette), so a band keeps its colour in the
// list under the graph. One hue per band, from the soft family of
// ChannelPalette, the same in both colour schemes.
QtObject {
    readonly property var colors: [
        Qt.rgba(0.93, 0.47, 0.45, 1),  // coral
        Qt.rgba(0.95, 0.64, 0.36, 1),  // orange
        Qt.rgba(0.92, 0.79, 0.40, 1),  // amber
        Qt.rgba(0.55, 0.80, 0.52, 1),  // sage
        Qt.rgba(0.36, 0.77, 0.68, 1),  // teal
        Qt.rgba(0.44, 0.68, 0.94, 1),  // sky
        Qt.rgba(0.58, 0.60, 0.94, 1),  // periwinkle
        Qt.rgba(0.73, 0.57, 0.92, 1),  // lavender
        Qt.rgba(0.89, 0.54, 0.72, 1),  // rose
        Qt.rgba(0.80, 0.62, 0.50, 1)   // clay
    ]

    readonly property int revision: PeqEditorController.revision
    property int listHovered: -1
    onListHoveredChanged: PeqEditorController.listHovered = listHovered

    function isSelected(band) { return PeqEditorController.isSelected(band) }
    function bandColor(band) { return colors[((band % colors.length) + colors.length) % colors.length] }
    function live(band) { return PeqEditorController.live(band) }
    function listClick(band, modifiers) { PeqEditorController.listClick(band, modifiers) }

    // The list hovering a row while nothing edits on the graph gives none back
    readonly property Connections editor: Connections {
        target: PeqEditorController
        function onListHoveredChanged() {
            if (PeqEditorController.listHovered !== listHovered) listHovered = PeqEditorController.listHovered
        }
    }
}
