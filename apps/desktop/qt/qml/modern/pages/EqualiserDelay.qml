import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- Equaliser > Delay & Gain: every output's gain, delay and the
// distance it stands for, an arrival bar for time alignment, and its polarity,
// mute and limiter, in one table. A row's output is the selected channel.
// -- WP UA1 -- (UXP A1, C5) a row names its output name first, number second
// ("Left woofer · 1"); the outputs nothing is routed to fold under one row.
EqualiserSection {
    id: page
    objectName: "EqualiserDelay"
    channelsShown: false

    readonly property var delays: {
        const list = []
        if (ChannelController.revision < 0) return list
        for (let o = 0; o < ChannelController.numOutputs; ++o) list.push(ChannelController.output(o))
        return list
    }
    // -- WP UA1 -- the outputs the table shows: every one while the unused are
    // open, else those with a route, those a test signal plays on (the core's
    // `hidden` is unused and silent: a playing output never folds) and the selected one
    property bool unusedOpen: false
    readonly property var unusedOutputs: {
        const list = []
        if (DeviceModel.revision < 0 || ChannelController.revision < 0) return list
        for (let o = 0; o < ChannelController.numOutputs; ++o) {
            const r = DeviceModel.channels.rowOfChannel(ChannelController.outputChannel(o))
            if (r >= 0 && DeviceModel.channels.get(r, "hidden") === true) list.push(o)
        }
        return list
    }
    readonly property var shownOutputs: {
        const list = []
        for (let o = 0; o < ChannelController.numOutputs; ++o)
            if (unusedOpen || unusedOutputs.indexOf(o) < 0 || ChannelController.outputChannel(o) === channel) list.push(o)
        return list
    }
    // The latest enabled output, for the arrival bars
    readonly property real latest: Math.max(0.5, ...delays.filter(o => o.enabled === true).map(o => o.delay || 0))

    inspector: Component {
        ChannelInspector {
            channel: page.channel
            host: page.host
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radius
        color: Theme.panel
        clip: true
        Accessible.role: Accessible.Table
        Accessible.name: qsTr("Delay and gain")

        Flickable {
            id: flick
            objectName: "EqualiserDelay.scroll"
            anchors.fill: parent
            contentWidth: width
            contentHeight: table.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            ScrollBar.vertical: AppScrollBar {}

            Column {
                id: table
                width: flick.width

                RowLayout {
                    x: Theme.spacing
                    width: parent.width - 2 * Theme.spacing
                    height: 28
                    spacing: Theme.spacingSmall
                    component Heading: Text {
                        font: Theme.fontHeading
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                    Heading { text: qsTr("OUTPUT"); Layout.fillWidth: true; Layout.minimumWidth: 110; Layout.preferredWidth: 150; Layout.maximumWidth: 260 }
                    Heading { text: qsTr("ON"); Layout.preferredWidth: 40 }
                    Heading { text: qsTr("GAIN"); Layout.preferredWidth: 84 }
                    Heading { text: qsTr("DELAY"); Layout.preferredWidth: 88 }
                    Heading { text: qsTr("DISTANCE"); Layout.preferredWidth: 88 }
                    Heading { text: qsTr("ARRIVAL"); Layout.fillWidth: true; Layout.minimumWidth: 50; Layout.preferredWidth: 160 }
                    Heading { text: qsTr("POLARITY"); Layout.preferredWidth: 84 }
                    Heading { text: qsTr("MUTE"); Layout.preferredWidth: 36 }
                    Heading { text: qsTr("LIMITER"); Layout.preferredWidth: 96 }
                }
                Repeater {
                    model: ChannelController.numOutputs
                    EqualiserDelayRow {
                        required property int index
                        width: table.width
                        output: index
                        place: page.shownOutputs.indexOf(index)   // -- WP UA1 --
                        visible: place >= 0
                        latest: page.latest
                        selected: channel === page.channel
                        onLimiterWanted: {
                            SidebarController.select(channel)
                            page.showLimiter()
                        }
                    }
                }
                // -- WP UA1 -- (C5) the unused outputs, folded under one row
                EqualiserUnusedRow {
                    objectName: "EqualiserDelay.unused"
                    width: table.width
                    count: page.unusedOutputs.filter(o => page.unusedOpen || page.shownOutputs.indexOf(o) < 0).length
                    open: page.unusedOpen
                    onClicked: page.unusedOpen = !page.unusedOpen
                }
                Text {
                    x: Theme.spacing
                    width: parent.width - 2 * Theme.spacing
                    topPadding: 8
                    bottomPadding: 8
                    text: qsTr("The distance is what the delay stands for at 343 m/s: typing one sets the delay. The latest output has the longest bar. Measure can find the delays by itself (Auto-tune).")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
