import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The sidebar: INPUTS and OUTPUTS, one row per channel, and the device button
// at the bottom (ContentView.swift's sidebar). A first version: the rows show
// the device's names and select a channel; renaming, the curve pills, the row
// menus, the quick strip and the global controls come with the sidebar's own
// work package.
Rectangle {
    id: sidebar
    objectName: "Sidebar"
    color: Theme.sidebar
    property int selectedChannel: -1
    signal channelSelected(int channel)

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ListView {
            id: list
            objectName: "Sidebar.list"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: DeviceModel.channels
            section.property: "kind"
            section.delegate: Text {
                required property string section
                leftPadding: Theme.spacing
                topPadding: Theme.spacing
                bottomPadding: 4
                text: section === "input" ? qsTr("INPUTS") : qsTr("OUTPUTS")
                font: Theme.fontHeading
                color: Theme.textMuted
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            delegate: ItemDelegate {
                id: row
                required property string kind
                required property int number
                required property int channel
                required property string label
                required property string descriptor
                required property bool outputEnabled
                required property bool pdm
                objectName: "Sidebar.channel." + channel
                // What the row shows, for the smoke driver
                readonly property string shownName: label
                width: ListView.view.width
                visible: kind === "input" || outputEnabled
                height: visible ? Theme.rowHeight : 0
                highlighted: channel === sidebar.selectedChannel
                onClicked: sidebar.channelSelected(highlighted ? -1 : channel)

                contentItem: RowLayout {
                    spacing: Theme.spacingSmall
                    Rectangle {
                        width: 4
                        height: 14
                        radius: 2
                        color: ChannelPalette.forRow(row.kind, row.number, row.pdm)
                    }
                    Text {
                        Layout.fillWidth: true
                        text: row.label
                        font: Theme.fontBody
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Pill {
                        text: row.descriptor
                        tint: ChannelPalette.forRow(row.kind, row.number, row.pdm)
                    }
                }
                background: Rectangle {
                    color: row.highlighted ? Theme.selection : row.hovered ? Theme.controlHover : "transparent"
                }

                Accessible.role: Accessible.ListItem
                Accessible.name: label + ", " + descriptor
                Accessible.selected: highlighted
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.separator
        }

        DeviceStatus {
            Layout.fillWidth: true
            Layout.margins: Theme.spacingSmall
        }
    }
}
