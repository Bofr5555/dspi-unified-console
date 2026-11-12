import QtQuick
import QtQuick.Layouts
import UConsole

// One Overview card (the Mac's stereo and mono dashboard cards): a column per
// channel, side by side, each with its heading, delay readout, meter and the
// channel's ten PEQ bands as read-only rows. `entry` is one of
// DashboardController.cards. The layout gear shows at the top right while the
// pointer is over the card, and the right-most delay readout makes room for it.
Rectangle {
    id: card
    property var entry: ({ columns: [] })
    readonly property var columns: entry.columns || []
    readonly property bool gearShown: hover.hovered || layoutPopup.opened

    readonly property int headerHeight: 32
    readonly property int meterStripHeight: 12
    readonly property int rowHeight: 24
    readonly property int bandRows: 10
    // Room kept at the end of the right-most heading while the gear shows
    readonly property int gearSlot: 18

    implicitHeight: headerHeight + 1 + meterStripHeight + bandRows * rowHeight
    implicitWidth: 220 * Math.max(1, columns.length)
    radius: 10
    color: Theme.control
    clip: false

    function tint(column) {
        return ChannelPalette.forRow(column.output ? "output" : "input", column.number, column.pdm)
    }
    function faint(c, alpha) {
        return Qt.rgba(c.r, c.g, c.b, alpha)
    }

    HoverHandler { id: hover }

    // The edge in each column's colour, the left half and the right half
    Repeater {
        model: card.columns.length
        Item {
            required property int index
            x: index * card.width / card.columns.length
            width: card.width / card.columns.length
            height: card.height
            clip: true
            Rectangle {
                x: -parent.x
                width: card.width
                height: card.height
                radius: card.radius
                color: "transparent"
                border.width: 1
                border.color: card.faint(card.tint(card.columns[parent.index]), 0.3)
            }
        }
    }

    Row {
        id: columnsRow
        anchors.fill: parent

        Repeater {
            model: card.columns
            Item {
                id: column
                required property var modelData
                required property int index
                readonly property bool last: index === card.columns.length - 1
                readonly property color tone: card.tint(modelData)
                objectName: "Dashboard.column." + modelData.channel
                property string shownName: modelData.heading
                width: card.width / card.columns.length
                height: card.height

                // The divider between two columns
                Rectangle {
                    visible: column.index > 0
                    width: 1
                    height: parent.height
                    color: Theme.separator
                }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.leftMargin: column.index > 0 ? 1 : 0
                    spacing: 0

                    // Heading: colour dot, name, delay
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.preferredHeight: card.headerHeight
                        Layout.leftMargin: 8
                        Layout.rightMargin: 8 + (column.last && card.gearShown ? card.gearSlot : 0)
                        Behavior on Layout.rightMargin { NumberAnimation { duration: 150; easing.type: Easing.InOutQuad } }
                        spacing: 6
                        Rectangle {
                            implicitWidth: 6
                            implicitHeight: 6
                            radius: 3
                            color: column.tone
                        }
                        Text {
                            Layout.fillWidth: true
                            text: column.modelData.heading
                            font.pixelSize: 11
                            font.weight: Font.Bold
                            color: Theme.textMuted
                            elide: Text.ElideRight
                            Accessible.role: Accessible.Heading
                            Accessible.name: text
                        }
                        Text {
                            objectName: "Dashboard.delay." + column.modelData.channel
                            visible: text.length > 0
                            text: column.modelData.delay
                            font.family: Theme.fontMono.family
                            font.pixelSize: 9
                            color: Theme.textMuted
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 1
                        color: Theme.separator
                    }

                    // The channel's level, fed by the meter poll itself
                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: card.meterStripHeight
                        Meter {
                            objectName: "Dashboard.meter." + column.modelData.channel
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            height: 6
                            channel: column.modelData.channel
                            color: column.tone
                            trackColor: card.faint(Theme.text, 0.08)
                            clipColor: Theme.danger
                            clipping: column.modelData.channel < 32 && ((DashboardController.clipLatched >>> column.modelData.channel) & 1) === 1
                            opacity: column.modelData.muted ? 0.4 : 1
                            Accessible.role: Accessible.ProgressBar
                            Accessible.name: qsTr("%1 level").arg(column.modelData.heading)
                        }
                    }

                    // The ten bands
                    Repeater {
                        model: BandListModel { channel: column.modelData.channel }
                        DashboardRow {
                            Layout.fillWidth: true
                            Layout.preferredHeight: card.rowHeight
                            tone: column.tone
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }
        }
    }

    // The layout gear: faded rather than removed, so the popup keeps its anchor
    IconButton {
        id: gear
        objectName: "Dashboard.gear"
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 6
        anchors.rightMargin: 4
        iconName: "settings"
        iconSize: 10
        iconColor: layoutPopup.opened ? Theme.text : Theme.textMuted
        toolTip: qsTr("Dashboard layout")
        opacity: card.gearShown ? 1 : 0
        enabled: card.gearShown
        Behavior on opacity { NumberAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        onClicked: layoutPopup.opened ? layoutPopup.close() : layoutPopup.open()

        DashboardLayoutPopup {
            id: layoutPopup
            x: gear.width / 2 - width / 2
            y: gear.height + 4
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: columns.map(c => c.heading).join(" / ")

    // One band, read-only (the Mac's DashboardRow): number, type code, then
    // frequency, gain where the type has one and Q for a bell; a dash when off
    component DashboardRow: Rectangle {
        id: row
        required property int band
        required property int type
        required property real freq
        required property real gain
        required property real q
        property color tone
        readonly property bool active: type !== 0

        color: band % 2 === 0 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 4

            Text {
                Layout.preferredWidth: 14
                text: row.band + 1
                font.family: Theme.fontMono.family
                font.pixelSize: 10
                color: Theme.textMuted
            }
            Text {
                Layout.preferredWidth: 28
                text: DashboardController.typeCode(row.type)
                font.pixelSize: 10
                font.weight: Font.Bold
                color: row.active ? row.tone : Theme.textDisabled
            }
            Item { Layout.fillWidth: true }

            RowLayout {
                visible: row.active
                spacing: 2
                Text {
                    text: DashboardController.freqText(row.freq)
                    font.family: Theme.fontMono.family
                    font.pixelSize: 10
                    color: Theme.text
                }
                Text {
                    text: qsTr("Hz")
                    font.pixelSize: 8
                    color: Theme.textMuted
                }
                Text {
                    visible: DashboardController.usesGain(row.type)
                    Layout.leftMargin: 4
                    text: DashboardController.gainText(row.gain)
                    font.family: Theme.fontMono.family
                    font.pixelSize: 10
                    color: Theme.text
                }
                Text {
                    visible: DashboardController.usesGain(row.type)
                    text: qsTr("dB")
                    font.pixelSize: 8
                    color: Theme.textMuted
                }
                Text {
                    visible: DashboardController.showsQ(row.type)
                    Layout.leftMargin: 4
                    text: DashboardController.qText(row.q)
                    font.family: Theme.fontMono.family
                    font.pixelSize: 10
                    color: Theme.text
                }
                Text {
                    visible: DashboardController.showsQ(row.type)
                    text: qsTr("Q")
                    font.pixelSize: 8
                    color: Theme.textMuted
                }
            }
            Text {
                visible: !row.active
                text: "—"
                font.pixelSize: 10
                color: Theme.textDisabled
            }
        }

        Accessible.role: Accessible.Row
        Accessible.name: row.active
                         ? qsTr("Band %1, %2, %3 Hz").arg(row.band + 1).arg(DashboardController.typeCode(row.type))
                               .arg(DashboardController.freqText(row.freq))
                         : qsTr("Band %1, off").arg(row.band + 1)
    }
}
