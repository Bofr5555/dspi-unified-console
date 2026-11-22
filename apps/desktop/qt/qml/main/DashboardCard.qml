import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// One Overview card (the Mac's stereo and mono dashboard cards): a column per
// channel, side by side, each with its heading, delay readout, meter and the
// channel's ten PEQ bands as read-only rows. `entry` is one of
// DashboardController.cards. The layout gear shows at the top right while the
// pointer is over the card, and the right-most delay readout makes room for it.
//
// The edge runs from the left column's colour to the right one's between 40 %
// and 60 % of the width, as the Mac draws it: the card is that gradient, and
// the face is laid one pixel inside it.
Rectangle {
    id: card
    property var entry: ({ columns: [] })
    readonly property var columns: entry.columns || []
    // -- WP LAY5a -- a column opens its channel (Modern's Overview), in place of the gear
    property bool choosable: false
    signal chosen(int channel)
    readonly property bool gearShown: !choosable && (hover.hovered || layoutPopup.opened)

    readonly property int headerHeight: 32
    readonly property int rowHeight: 24
    // Room kept at the end of the right-most heading while the gear shows
    readonly property int gearSlot: 18
    readonly property int fadeMs: 150
    readonly property font nameFont: Qt.font({ pixelSize: 11, weight: Font.Bold })
    readonly property font readoutFont: Qt.font({ family: Theme.fontMono.family, pixelSize: 9 })

    function tint(column) {
        return ChannelPalette.forRow(column.output ? "output" : "input", column.number, column.pdm)
    }
    function faint(c, alpha) {
        return Qt.rgba(c.r, c.g, c.b, alpha)
    }
    // -- WP QP18 -- A neutral edge: the channel's colour is the heading's dot
    // only (docs/UI-MODES.md §7, small tags)
    function edge(at) {
        return Theme.separator
    }

    implicitHeight: headerHeight + 1 + 12 + 10 * rowHeight
    implicitWidth: 220 * Math.max(1, columns.length)
    radius: 10
    gradient: Gradient {
        orientation: Gradient.Horizontal
        GradientStop { position: 0.4; color: card.edge(0) }
        GradientStop { position: 0.6; color: card.edge(1) }
    }

    HoverHandler { id: hover }

    Rectangle {
        id: face
        x: 1
        y: 1
        width: card.width - 2
        height: card.height - 2
        radius: card.radius - 1
        color: Theme.control

        Row {
            width: face.width
            height: face.height

            Repeater {
                model: card.columns
                ColumnLayout {
                    id: column
                    required property var modelData
                    required property int index
                    readonly property bool last: index === card.columns.length - 1
                    readonly property color tone: card.tint(modelData)
                    objectName: "Dashboard.column." + modelData.channel
                    property string shownName: modelData.heading
                    width: face.width / card.columns.length
                    height: face.height
                    spacing: 0

                    // Heading: colour dot, name, delay
                    RowLayout {
                        Layout.preferredHeight: card.headerHeight
                        Layout.leftMargin: 8
                        Layout.rightMargin: 8 + (column.last && card.gearShown ? card.gearSlot : 0)
                        Behavior on Layout.rightMargin { NumberAnimation { duration: card.fadeMs } }
                        spacing: 6
                        Rectangle {
                            implicitWidth: 6
                            implicitHeight: 6
                            radius: 3
                            color: column.tone
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: column.modelData.heading
                            font: card.nameFont
                            color: Theme.textMuted
                            elide: Text.ElideRight
                            Accessible.role: Accessible.Heading
                            Accessible.name: text
                        }
                        Text {
                            objectName: "Dashboard.delay." + column.modelData.channel
                            visible: text.length > 0
                            text: column.modelData.delay
                            font: card.readoutFont
                            color: Theme.textMuted
                        }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 1
                        color: Theme.separator
                    }

                    // The channel's level, fed by the meter poll itself
                    Meter {
                        objectName: "Dashboard.meter." + column.modelData.channel
                        Layout.fillWidth: true
                        Layout.preferredHeight: 6
                        Layout.margins: 8
                        Layout.topMargin: 3
                        Layout.bottomMargin: 3
                        channel: column.modelData.channel
                        color: Theme.meterLevel   // -- WP QP18 --
                        trackColor: card.faint(Theme.text, 0.08)
                        clipColor: Theme.danger
                        clipping: column.modelData.channel < 32
                                  && ((DashboardController.clipLatched >>> column.modelData.channel) & 1) !== 0
                        opacity: column.modelData.muted ? 0.4 : 1
                        Accessible.role: Accessible.ProgressBar
                        Accessible.name: qsTr("%1 level").arg(column.modelData.heading)
                    }

                    // The ten bands
                    Repeater {
                        model: BandListModel { channel: column.modelData.channel }
                        DashboardRow {
                            Layout.preferredHeight: card.rowHeight
                            tone: Theme.text   // -- WP QP18 -- band codes in text colour
                        }
                    }
                    Item { Layout.fillHeight: true }
                }
            }
        }

        // -- WP LAY5a -- Each column is a button (Modern's Overview). Over the
        // columns' content, which has nothing to click.
        Row {
            objectName: "DashboardCard.buttons"
            visible: card.choosable
            anchors.fill: parent
            Repeater {
                model: card.choosable ? card.columns : []
                AbstractButton {
                    id: choose
                    required property var modelData
                    objectName: "Dashboard.open." + modelData.channel
                    width: face.width / card.columns.length
                    height: face.height
                    hoverEnabled: true
                    focusPolicy: Qt.TabFocus
                    onClicked: card.chosen(modelData.channel)
                    background: Rectangle {
                        radius: card.radius - 1
                        color: choose.hovered ? card.faint(Theme.text, 0.05) : "transparent"
                        border.width: choose.visualFocus ? 2 : 0
                        border.color: Theme.focusRing
                    }
                    Accessible.role: Accessible.Button
                    //: %1 is a channel's name
                    Accessible.name: qsTr("Open %1 in the Equaliser").arg(modelData.heading)
                    Accessible.onPressAction: choose.clicked()
                }
            }
        }
        // Dividers between the columns
        Repeater {
            model: Math.max(0, card.columns.length - 1)
            Rectangle {
                required property int index
                x: (index + 1) * face.width / card.columns.length
                width: 1
                height: face.height
                color: Theme.separator
            }
        }
    }

    // The layout gear: faded rather than removed, so the popup keeps its anchor
    IconButton {
        id: gear
        objectName: "Dashboard.gear"
        x: card.width - width - 4
        y: 6
        iconName: "settings"
        iconSize: 10
        iconColor: layoutPopup.opened ? Theme.text : Theme.textMuted
        toolTip: qsTr("Dashboard layout")
        opacity: card.gearShown ? 1 : 0
        enabled: card.gearShown
        Behavior on opacity { NumberAnimation { duration: card.fadeMs } }
        onClicked: layoutPopup.opened ? layoutPopup.close() : layoutPopup.open()

        DashboardLayoutPopup {
            id: layoutPopup
            x: gear.width / 2 - width / 2
            y: gear.height + 4
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: columns.map(c => c.heading).join(" / ")

    // A number and its unit, as the Mac sets them: the unit smaller and muted
    component Reading: Row {
        property alias value: number.text
        property alias unit: unitText.text
        spacing: 2
        Text {
            id: number
            font.family: Theme.fontMono.family
            font.pixelSize: 10
            color: Theme.text
        }
        Text {
            id: unitText
            y: number.baselineOffset - baselineOffset
            font.pixelSize: 8
            color: Theme.textMuted
        }
    }

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

        Layout.fillWidth: true
        color: band % 2 === 0 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"

        Text {
            x: 8
            anchors.verticalCenter: parent.verticalCenter
            text: row.band + 1
            font.family: Theme.fontMono.family
            font.pixelSize: 10
            color: Theme.textMuted
        }
        Text {
            x: 26
            anchors.verticalCenter: parent.verticalCenter
            text: DashboardController.typeCode(row.type)
            font.pixelSize: 10
            font.bold: true
            color: row.active ? row.tone : Theme.textDisabled
        }
        Row {
            x: row.width - width - 8
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6
            Reading {
                visible: row.active
                value: DashboardController.freqText(row.freq)
                unit: qsTr("Hz")
            }
            Reading {
                visible: row.active && DashboardController.usesGain(row.type)
                value: DashboardController.gainText(row.gain)
                unit: qsTr("dB")
            }
            Reading {
                visible: row.active && DashboardController.showsQ(row.type)
                value: DashboardController.qText(row.q)
                unit: qsTr("Q")
            }
            Text {
                visible: !row.active
                text: "—"
                font: Theme.fontCaption
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
