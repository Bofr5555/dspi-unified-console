import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- An output's page under the graph (the Mac's OutputChannelDetail):
// its name and enable switch, the output card (between them where the page is
// wide enough, QTTWEAK), then its band list with the
// PEQ / XO tabs (crossovers from wire format V11). Both tabs are a FilterList;
// the XO tab lists the four crossover bands. While the XO tab shows,
// ChannelController.crossoverTabShown is set for the graph.
//
//     OutputDetail { output: 0; graphSelection: ... }
ColumnLayout {
    id: page
    property int output: 0
    property var graphSelection: null
    property bool crossoverTab: false

    readonly property int channel: (ChannelController.revision < 0 ? undefined : ChannelController.outputChannel(output))
    readonly property int row: (DeviceModel.revision < 0 ? undefined : DeviceModel.channels.rowOfChannel(channel))
    readonly property string label: row >= 0 ? DeviceModel.channels.get(row, "label") : qsTr("Out %1").arg(output + 1)
    readonly property bool pdm: row >= 0 && DeviceModel.channels.get(row, "pdm") === true
    readonly property bool enabledOut: (ChannelController.revision < 0 ? undefined : ChannelController.output(output).enabled === true)
    readonly property bool showsCrossover: crossoverTab && ChannelController.crossoverSupported

    objectName: "OutputDetail"
    spacing: Theme.spacing

    // A device without crossovers goes back to the PEQ tab
    Connections {
        target: ChannelController
        function onChanged() { if (!ChannelController.crossoverSupported) page.crossoverTab = false }
        function onCore1Question(output, title, text, acceptText) {
            if (output !== page.output) return
            core1Question.title = title
            core1Question.text = text
            core1Question.acceptText = acceptText
            core1Question.open()
        }
    }
    onShowsCrossoverChanged: ChannelController.crossoverTabShown = showsCrossover
    Component.onCompleted: ChannelController.crossoverTabShown = showsCrossover
    Component.onDestruction: ChannelController.crossoverTabShown = false

    // -- WP QP9 -- The spectrum as bars, at the top of the page
    SpectrumStrip {
        place: "page"
        Layout.fillWidth: true
    }

    // The name and the enable switch, with the output card between them where
    // the page is wide enough for the card in one row (-- WP QTTWEAK --, it
    // was the row's empty middle); otherwise the card goes under them
    GridLayout {
        id: header
        // A long name elides beside the card rather than push it out
        readonly property real titleWidth: Math.min(titleRow.implicitWidth, 260)
        readonly property bool holdsCard: page.width >= titleWidth + outputCard.oneRowWidth
                                                         + enableSwitch.implicitWidth + 2 * columnSpacing
        Layout.fillWidth: true
        columns: 3
        columnSpacing: Theme.spacing
        rowSpacing: page.spacing

        RowLayout {
            id: titleRow
            Layout.row: 0
            Layout.column: 0
            Layout.fillWidth: !header.holdsCard
            Layout.preferredWidth: header.holdsCard ? header.titleWidth : -1
            spacing: Theme.spacingSmall
            Pill {
                text: "OUT" + (page.output + 1)
                tint: ChannelPalette.output(page.output, page.pdm)
            }
            Text {
                objectName: "OutputDetail.title"
                Layout.fillWidth: true
                text: page.label
                font: Theme.fontTitle
                color: Theme.text
                elide: Text.ElideRight
            }
        }
        AppSwitch {
            id: enableSwitch
            objectName: "OutputDetail.enable"
            Layout.row: 0
            Layout.column: 2
            text: qsTr("Enabled")
            enabled: ChannelController.writable
            checked: page.enabledOut
            onToggled: {
                // A refused or asked-about enable shows the device's state again
                ChannelController.requestOutputEnable(page.output, checked)
                checked = Qt.binding(() => page.enabledOut)
            }
        }
        OutputCard {
            id: outputCard
            Layout.row: header.holdsCard ? 0 : 1
            Layout.column: header.holdsCard ? 1 : 0
            Layout.columnSpan: header.holdsCard ? 1 : 3
            Layout.fillWidth: true
            output: page.output
        }
    }

    FilterList {
        objectName: "OutputDetail.bands"
        visible: !page.showsCrossover
        Layout.fillWidth: true
        Layout.fillHeight: true
        channel: page.channel
        includeLinkwitz: true
        showTabs: ChannelController.crossoverSupported
        crossoverTab: false
        graphSelection: page.graphSelection
        onTabChosen: xo => page.crossoverTab = xo
    }
    FilterList {
        objectName: "OutputDetail.crossover"
        visible: page.showsCrossover
        Layout.fillWidth: true
        Layout.fillHeight: true
        channel: page.channel
        crossover: true
        showTabs: true
        crossoverTab: true
        onTabChosen: xo => page.crossoverTab = xo
    }

    QuestionDialog {
        id: core1Question
        objectName: "OutputDetail.core1Question"
        parent: Overlay.overlay
        onAnswered: answer => { if (answer === "accept") ChannelController.confirmOutputEnable(page.output) }
    }

    Accessible.role: Accessible.Pane
    Accessible.name: label
}
