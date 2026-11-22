import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The dashboard layout (the Mac's DashboardLayoutPanel): Auto, or a fixed
// number of cards per row.
//
// -- WP LAY2 -- The content alone: Classic shows it in the popover every
// dashboard card's gear opens (DashboardLayoutPopup), Modern on its Overview
// page.
ColumnLayout {
    id: content
    Layout.fillWidth: true

    // A fixed choice's name ("1 card per row", "2 cards per row")
    function perRow(n) {
        return n === 1 ? qsTr("1 card per row") : qsTr("%1 cards per row").arg(n)
    }

    // The controller holds the choice to 0..3
    readonly property int chosen: DashboardController.cardsPerRow

    spacing: 0

    Text {
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Layout.topMargin: 12
        Layout.bottomMargin: 8
        text: qsTr("DASHBOARD LAYOUT")
        font.pixelSize: 10
        font.weight: Font.Bold
        color: Theme.textMuted
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }

    RowLayout {
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Layout.fillWidth: true
        spacing: 6
        Repeater {
            model: 4
            LayoutTile {
                required property int index
                count: index
                chosen: content.chosen
                tip: index === 0 ? qsTr("Fit cards to the window width") : content.perRow(index)
                Layout.fillWidth: true
            }
        }
    }

    Text {
        Layout.fillWidth: true
        Layout.margins: 12
        objectName: "DashboardLayoutPopup.summary"
        text: content.chosen === 0 ? qsTr("Fits as many cards per row as the window allows.")
                                 : content.chosen === 1 ? qsTr("Up to 1 card per row.")
                                                      : qsTr("Up to %1 cards per row.").arg(content.chosen)
        font.pixelSize: 10
        color: Theme.textMuted
        wrapMode: Text.WordWrap
    }

    // One choice: a picture of the layout and its name
    component LayoutTile: AbstractButton {
        id: tile
        property int count
        property int chosen
        property string tip
        readonly property bool on: chosen === count
        readonly property color tone: on ? Theme.accentText : Theme.textMuted
        objectName: "DashboardLayoutPopup.tile." + count
        implicitHeight: 38
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        checkable: false
        onClicked: DashboardController.cardsPerRow = count

        contentItem: ColumnLayout {
            spacing: 4
            Item {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: 22
                implicitHeight: 13
                Icon {
                    visible: tile.count === 0
                    anchors.centerIn: parent
                    name: "move-horizontal"
                    size: 13
                    color: tile.tone
                }
                // Two rows of rounded cells, `count` to a row
                Column {
                    visible: tile.count > 0
                    anchors.fill: parent
                    spacing: 2
                    Repeater {
                        model: 2
                        Row {
                            spacing: 2
                            width: parent.width
                            height: (parent.height - parent.spacing) / 2
                            Repeater {
                                model: tile.count
                                Rectangle {
                                    width: (22 - (tile.count - 1) * 2) / tile.count
                                    height: parent.height
                                    radius: 2
                                    color: tile.tone
                                }
                            }
                        }
                    }
                }
            }
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: tile.count === 0 ? qsTr("Auto") : String(tile.count)
                font.pixelSize: 9
                font.bold: tile.on
                color: tile.tone
            }
        }
        background: Rectangle {
            radius: 6
            color: tile.on ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.16)
                           : Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, tile.hovered ? 0.09 : 0.05)
            border.width: tile.on || tile.visualFocus ? 1 : 0
            border.color: tile.visualFocus ? Theme.focusRing : Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.7)
        }

        HelpTip {
            text: tile.tip
            visible: tile.hovered
        }

        Accessible.role: Accessible.RadioButton
        Accessible.name: tile.count === 0 ? qsTr("Auto") : tile.tip
        Accessible.checked: tile.on
        Accessible.onPressAction: tile.clicked()
    }
}
