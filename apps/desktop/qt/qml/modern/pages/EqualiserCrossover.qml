import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- Equaliser > Crossover: every enabled output's curve over one
// table of all high and low passes; under it the selected output's four
// crossover bands, with their bypass.
// -- WP DL8 -- (DESREV §5) the graph in a GraphFrame (labels inside, the grid
// BodePlot's own), the table striped; the four bands only where the table
// cannot show them: a family other than Linkwitz-Riley, or a second high or
// low pass. The inspector has the rest (its HP/LP).
EqualiserSection {
    id: page
    objectName: "EqualiserCrossover"

    readonly property bool supported: ChannelController.crossoverSupported
    // -- WP DL8 -- the selected output's bands need their own list
    readonly property bool detailed: {
        if (!isOutput || channel < 0 || ChannelController.revision < 0) return false
        const types = []
        for (let b = 0; b < 4; ++b) types.push(ChannelController.band(channel, b, true).type)
        const used = types.filter(t => t !== 0)
        const lows = used.filter(t => ChannelController.isLowPass(t)).length
        return used.some(t => ChannelController.familyOf(t) !== 1) || lows > 1 || used.length - lows > 1
    }
    readonly property var curves: {
        const list = []
        if (ChannelController.revision < 0) return list
        for (let o = 0; o < ChannelController.numOutputs; ++o)
            if (ChannelController.output(o).enabled === true) list.push(ChannelController.outputChannel(o))
        return list
    }
    readonly property var colors: {
        const out = []
        if (DeviceModel.revision < 0) return out
        for (let r = 0; r < DeviceModel.channels.count; ++r)
            out[DeviceModel.channels.get(r, "channel")] = ChannelPalette.forRow(DeviceModel.channels.get(r, "kind"),
                DeviceModel.channels.get(r, "number"), DeviceModel.channels.get(r, "pdm"))
        return out
    }

    inspector: Component {
        ChannelInspector {
            channel: page.channel
            host: page.host
        }
    }

    // -- WP DL8 -- EmptyState for the bare sentence
    EmptyState {
        objectName: "EqualiserCrossover.unsupported"
        anchors.fill: parent
        visible: !page.supported
        iconName: "chart-line"
        title: qsTr("No crossovers")
        text: qsTr("This firmware has no crossovers.")
    }

    ColumnLayout {
        id: column
        visible: page.supported
        anchors.fill: parent
        spacing: Theme.spacingSmall

        // -- WP DL8 -- GraphFrame: no outline, the labels whole inside; its grid
        // off, BodePlot draws the grid itself (the frame's lines would run
        // under the labels: lay7's contrast check, DL1/DL2)
        GraphFrame {
            id: plot
            objectName: "EqualiserCrossover.plot"
            Layout.fillWidth: true
            // The table whole where it fits, the rest the graph's; else the
            // graph's own height, as far as the lists keep a few rows
            Layout.preferredHeight: Math.max(160, column.height - column.spacing - lists.implicitHeight,
                                             Math.min(GraphController.graphHeight, column.height - 180))
            grid: false
            xTicks: frequencyTicks(GraphController.minFreq, GraphController.maxFreq, GraphController.showFrequencyLabels)
            yTicks: dbTicks(GraphController.dbTop, GraphController.dbBottom, GraphController.dbRange, GraphController.showDBLabels)
            rightTicks: phaseTicks(GraphController.phaseTop, GraphController.showPhase)
            Accessible.name: qsTr("Every output's response")

            // As GraphPlot's (Modern), so the two graphs read the same
            function frequencyTicks(min, max, shown) {
                const out = []
                if (!shown || !(min > 0) || !(max > min)) return out
                for (const decade of [10, 100, 1000, 10000])
                    for (const k of [1, 2, 5]) {
                        const hz = k * decade
                        if (hz < 20 || hz > 20000 || hz < min || hz > max) continue
                        out.push({ at: Math.log10(hz / min) / Math.log10(max / min), text: hz < 1000 ? String(hz) : (hz / 1000) + "k" })
                    }
                return out
            }
            function dbTicks(top, bottom, span, shown) {
                const out = []
                if (!shown || !(top > bottom)) return out
                const step = span <= 12 ? 1 : span <= 30 ? 3 : span <= 60 ? 5 : 10
                for (let k = Math.ceil(bottom / step); k * step <= top; ++k) {
                    const db = k * step
                    out.push({ at: (top - db) / (top - bottom), text: (k >= 0 ? "+" : "") + db })
                }
                return out
            }
            function phaseTicks(extent, shown) {
                if (!shown) return []
                return [1, 0.5, 0, -0.5, -1].map(share => {
                    const deg = Math.round(share * extent)
                    return { at: (1 - share) / 2, text: deg === 0 ? "0°" : (deg > 0 ? "+" : "") + deg + "°" }
                })
            }

            BodePlot {
                objectName: "EqualiserCrossover.grid"
                anchors.fill: parent
                graph: GraphController
                layers: BodePlot.Grid
                gridColor: Theme.text
            }
            BodePlot {
                objectName: "EqualiserCrossover.curves"
                anchors.fill: parent
                graph: GraphController
                layers: BodePlot.Curves
                channels: page.curves
                activeChannel: page.isOutput ? page.channel : -1
                followsSelection: false
                excludeOverlay: false
                channelColors: page.colors
                phaseColor: Theme.text
                labelColor: Theme.textMuted
            }
        }

        Flickable {
            id: flick
            objectName: "EqualiserCrossover.scroll"
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: width
            contentHeight: lists.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            ScrollBar.vertical: AppScrollBar {}

            ColumnLayout {
                id: lists
                width: flick.width
                spacing: Theme.spacingSmall

                // ---- Every output's high and low pass ----
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: table.implicitHeight
                    radius: Theme.radius
                    color: Theme.panel
                    clip: true
                    Accessible.role: Accessible.Table
                    Accessible.name: qsTr("Crossovers")
                    Column {
                        id: table
                        readonly property real nameWidth: width < 620 ? 130 : 150
                        width: parent.width
                        // -- WP DL8 -- the table of DESIGN-LANGUAGE §9: a 24 px header over
                        // the one rule that stays, 28 px rows in stripes, the hover tone
                        RowLayout {
                            x: Theme.spacing
                            width: parent.width - 2 * Theme.spacing
                            height: 24
                            spacing: Theme.spacing
                            component Heading: Text {
                                font: Theme.fontHeading
                                color: Theme.textMuted
                                elide: Text.ElideRight
                            }
                            Heading { text: qsTr("OUTPUT"); Layout.preferredWidth: table.nameWidth }
                            Heading { text: qsTr("HIGH PASS"); Layout.fillWidth: true; Layout.preferredWidth: 1 }
                            Heading { text: qsTr("LOW PASS"); Layout.fillWidth: true; Layout.preferredWidth: 1 }
                        }
                        Rectangle { width: parent.width; height: 1; color: Theme.separator }
                        Repeater {
                            model: page.supported ? ChannelController.numOutputs : 0
                            Rectangle {
                                id: row
                                required property int index
                                readonly property int channel: ChannelController.revision < 0 ? -1 : ChannelController.outputChannel(index)
                                readonly property bool enabledOut: ChannelController.revision >= 0 && ChannelController.output(index).enabled === true
                                objectName: "EqualiserCrossover.row." + index
                                width: table.width
                                height: Theme.rowHeight
                                color: channel === page.channel ? Theme.selection : "transparent"
                                Rectangle {
                                    anchors.fill: parent
                                    visible: row.index % 2 === 0 && row.channel !== page.channel
                                    color: Theme.stripe
                                }
                                Rectangle {
                                    anchors.fill: parent
                                    visible: rowHover.hovered && row.channel !== page.channel
                                    color: Theme.controlHover
                                    opacity: 0.6
                                }
                                HoverHandler { id: rowHover }
                                TapHandler { onTapped: SidebarController.select(row.channel) }
                                ChannelInfo { id: info; channel: row.channel }
                                RowLayout {
                                    x: Theme.spacing
                                    width: parent.width - 2 * Theme.spacing
                                    height: parent.height
                                    spacing: Theme.spacing
                                    opacity: row.enabledOut ? 1 : Theme.dim(0.55)
                                    RowLayout {
                                        Layout.preferredWidth: table.nameWidth
                                        Layout.maximumWidth: table.nameWidth
                                        spacing: Theme.spacingSmall
                                        Pill {
                                            Layout.preferredWidth: 44
                                            text: info.descriptor
                                            tint: info.tint
                                        }
                                        Text {
                                            Layout.fillWidth: true
                                            textFormat: Text.PlainText
                                            text: info.label
                                            font: Theme.fontBody
                                            color: Theme.text
                                            elide: Text.ElideRight
                                        }
                                    }
                                    EqualiserCrossoverSide {
                                        objectName: row.objectName + ".highPass"
                                        Layout.fillWidth: true
                                        Layout.preferredWidth: 1
                                        channel: row.channel
                                        owner: info.label
                                        brief: true
                                        highPass: true
                                    }
                                    EqualiserCrossoverSide {
                                        objectName: row.objectName + ".lowPass"
                                        Layout.fillWidth: true
                                        Layout.preferredWidth: 1
                                        channel: row.channel
                                        owner: info.label
                                        brief: true
                                        highPass: false
                                    }
                                }
                                Accessible.role: Accessible.Row
                                Accessible.name: info.label
                            }
                        }
                    }
                }

                // -- WP DL8 -- help under the table (the short names in it)
                Text {
                    objectName: "EqualiserCrossover.legend"
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: qsTr("LR is Linkwitz-Riley, BW Butterworth, BS Bessel.")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }

                // ---- The selected output's four bands, -- WP DL8 -- where the
                // table cannot show them
                SectionHeading {
                    objectName: "EqualiserCrossover.bandsHeading"
                    visible: page.detailed
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.spacingSection - lists.spacing
                    //: %1 is an output's name ("Out 1")
                    text: qsTr("Crossover bands of %1").arg(named.label)
                    ChannelInfo { id: named; channel: page.channel }
                }
                FilterList {
                    objectName: "EqualiserCrossover.bands"
                    visible: page.detailed
                    Layout.fillWidth: true
                    Layout.preferredHeight: implicitHeight
                    rowHeight: Theme.rowHeight
                    flat: true
                    channel: page.detailed ? page.channel : -1
                    crossover: true
                }
            }
        }
    }
}
