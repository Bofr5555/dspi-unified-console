import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- Equaliser > Crossover: every enabled output's curve over one
// table of all high and low passes; under it the selected output's four
// crossover bands, with their bypass.
EqualiserSection {
    id: page
    objectName: "EqualiserCrossover"

    readonly property bool supported: ChannelController.crossoverSupported
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

    Text {
        anchors.centerIn: parent
        visible: !page.supported
        text: qsTr("This firmware has no crossovers.")
        font: Theme.fontTitle
        color: Theme.textMuted
    }

    ColumnLayout {
        id: column
        visible: page.supported
        anchors.fill: parent
        spacing: Theme.spacingSmall

        Rectangle {
            objectName: "EqualiserCrossover.plot"
            Layout.fillWidth: true
            // The lists keep room for a few rows
            Layout.preferredHeight: Math.max(120, Math.min(Math.max(GraphController.graphHeight, column.height - 14 * 32), column.height - 180))
            radius: Theme.radius
            color: Theme.panel
            clip: true
            Accessible.role: Accessible.Graphic
            Accessible.name: qsTr("Every output's response")

            BodePlot {
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
            BodePlot {
                anchors.fill: parent
                graph: GraphController
                layers: BodePlot.Labels
                labelColor: Theme.textMuted
                phaseColor: Theme.text
                labelFont: Theme.fontCaption
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
                        RowLayout {
                            x: Theme.spacing
                            width: parent.width - 2 * Theme.spacing
                            height: 26
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
                        Repeater {
                            model: page.supported ? ChannelController.numOutputs : 0
                            Rectangle {
                                id: row
                                required property int index
                                readonly property int channel: ChannelController.revision < 0 ? -1 : ChannelController.outputChannel(index)
                                readonly property bool enabledOut: ChannelController.revision >= 0 && ChannelController.output(index).enabled === true
                                objectName: "EqualiserCrossover.row." + index
                                width: table.width
                                height: 32
                                color: channel === page.channel ? Theme.selection : "transparent"
                                Rectangle {
                                    anchors.fill: parent
                                    visible: row.index % 2 === 0 && row.channel !== page.channel
                                    color: Theme.text
                                    opacity: 0.03
                                }
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

                // ---- The selected output's four bands ----
                Text {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    textFormat: Text.PlainText
                    text: page.isOutput ? qsTr("LR is Linkwitz-Riley, BW Butterworth, BS Bessel. The four crossover bands of %1:").arg(named.label)
                                        : qsTr("Inputs have no crossover. Select an output to see its four bands.")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                    ChannelInfo { id: named; channel: page.channel }
                }
                FilterList {
                    objectName: "EqualiserCrossover.bands"
                    visible: page.isOutput
                    Layout.fillWidth: true
                    Layout.preferredHeight: implicitHeight
                    rowHeight: 32
                    flat: true
                    channel: page.isOutput ? page.channel : -1
                    crossover: true
                }
            }
        }
    }
}
