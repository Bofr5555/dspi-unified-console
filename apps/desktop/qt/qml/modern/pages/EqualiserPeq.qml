import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- Equaliser > PEQ: the graph, where the bands are edited (drag a
// dot, the wheel for Q, a double-click for a new one), over the band table,
// whose every field Tab reaches. The inspector has the selected band and the
// channel. The band the keyboard or a click is at in the table is selected.
EqualiserSection {
    id: page
    objectName: "EqualiserPeq"

    readonly property bool bandBar: LayoutController.arrangement.bandBar === "shown"
    readonly property bool linked: !isOutput && channel >= 0 && ChannelController.revision >= 0 && ChannelController.linkedPartner(channel) >= 0

    // ---- The band on show: the graph's and the table's selection, or the
    // table's row of a band that is off and has no dot
    property int rowBand: -1
    readonly property var selected: PeqEditorController.selected
    readonly property int band: selected.length === 0 ? rowBand
                                : selected.indexOf(rowBand) >= 0 ? rowBand : selected[0]
    property bool hadSelection: false
    onSelectedChanged: {
        if (selected.length === 0) {
            if (hadSelection) rowBand = -1
        } else if (selected.indexOf(rowBand) < 0) {
            rowBand = selected[0]
        }
        hadSelection = selected.length > 0
    }
    // A row that was off has a dot now
    readonly property int rowType: ChannelController.revision < 0 || rowBand < 0 ? 0
                                   : ChannelController.band(channel, rowBand, false).type
    onRowTypeChanged: if (rowType > 0 && selected.length === 0) PeqEditorController.rowClicked(rowBand, 0)
    onChannelChanged: rowBand = -1
    Connections {
        target: DeviceRegistry
        function onActiveChanged() { page.rowBand = -1 }
    }
    function rowFocused(band) {
        const off = ChannelController.band(channel, band, false).type === 0
        if (off) PeqEditorController.forget()
        else if (!PeqEditorController.isSelected(band)) PeqEditorController.rowClicked(band, 0)
        rowBand = band
    }
    readonly property Item focusItem: shown ? Window.activeFocusItem : null
    onFocusItemChanged: {
        for (let i = focusItem; i && i !== bands; i = i.parent)
            if (i.bandColor !== undefined && i.band !== undefined && bands.isAncestor(i)) {
                rowFocused(i.band)
                return
            }
    }

    // Enter, or a double-click on a dot: type the band's values in the inspector
    Connections {
        target: page.shown && !page.bandBar && !GraphController.poppedOut ? graphHost.graph.peqEditor : null
        function onEditRequested(field) {
            InspectorState.setExpanded("band", true)
            InspectorState.show()
            Qt.callLater(page.typeValues, field)
        }
    }
    function typeValues(field) {
        const body = inspectorItem.body
        if (body && body.typeValues) body.typeValues(field)
    }

    inspector: Component {
        ChannelInspector {
            channel: page.channel
            host: page.host
            bandShown: true
            band: page.band
            bandsSelected: page.selected.length
        }
    }

    ColumnLayout {
        id: column
        anchors.fill: parent
        spacing: Theme.spacingSmall

        // What the graph may take: the table whole, and the rest is the graph's;
        // where that is too little, the height the graph was dragged to, as far
        // as the table keeps a few rows
        readonly property real bars: spectrum.visible ? spectrum.implicitHeight + spacing : 0
        readonly property real dock: page.bandBar ? Theme.controlHeight + 18 : 0
        readonly property real rest: height - bars - dock - spacing - bands.implicitHeight
        readonly property real most: height - bars - dock - 2 * spacing - 12 - bands.leastHeight
        readonly property bool roomy: rest >= GraphController.graphHeight

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: graphHost.implicitHeight
            GraphHost {
                id: graphHost
                width: parent.width
                selectedChannel: page.channel
                linkedPartner: page.isOutput || page.channel < 0 ? -1
                    : (ChannelController.revision < 0 ? -1 : ChannelController.linkedPartner(page.channel))
                plotHeight: Math.max(120, column.roomy ? column.rest : Math.min(GraphController.graphHeight, column.most))
                resizable: !column.roomy
                bandBar: page.bandBar
            }
            EqualiserGraphToggles {
                visible: !GraphController.poppedOut
                channel: graphHost.selectedChannel
                partner: graphHost.linkedPartner
                anchors { top: parent.top; right: parent.right; topMargin: 6; rightMargin: 40 }
            }
        }

        // -- WP QP9 -- The spectrum as bars
        SpectrumStrip {
            id: spectrum
            place: "page"
            Layout.fillWidth: true
        }

        FilterList {
            id: bands
            objectName: "EqualiserPeq.bands"
            readonly property real leastHeight: 26 + 4 * (rowHeight + 1) + 40
            function isAncestor(item) {
                for (let i = item; i; i = i.parent) if (i === bands) return true
                return false
            }
            Layout.fillWidth: true
            Layout.fillHeight: true
            rowHeight: 28
            flat: true
            channel: page.channel
            includeLinkwitz: page.isOutput   // a driver tool: outputs only
            clearsInput: !page.isOutput
            //: %1 is an input pair ("1/2")
            clearText: page.linked ? qsTr("Clear %1 PEQ").arg((page.channel & ~1) + 1 + "/" + ((page.channel | 1) + 1))
                                   : qsTr("Clear PEQ")
            graphSelection: PeqSelection
        }
    }

    Connections {
        target: page.shown ? PeqEditorController : null
        function onShowRow(band) { bands.showBand(band) }
    }
}
