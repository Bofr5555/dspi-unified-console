import QtQuick
import QtQuick.Layouts
import UConsole

// The right pane: the response graph on top and, below it, the overview
// (dashboard) or the selected channel's detail. Empty in the base: the graph,
// the dashboard and the channel pages come with their own work packages and
// replace the placeholders below.
Rectangle {
    id: pane
    objectName: "RightPane"
    color: Theme.window
    property int selectedChannel: -1

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacing

        // -- WP QP3 -- The response graph: header, graph, drag strip
        GraphHost {
            Layout.fillWidth: true
            selectedChannel: pane.selectedChannel
        }

        // The overview's or the channel detail's place
        Rectangle {
            objectName: "RightPane.detail"
            Layout.fillWidth: true
            Layout.fillHeight: true
            radius: Theme.radius
            color: Theme.panel
            border.width: 1
            border.color: Theme.separator

            Text {
                anchors.centerIn: parent
                visible: !DeviceModel.connected
                text: qsTr("No Device Connected")
                font: Theme.fontTitle
                color: Theme.textMuted
            }

            // -- WP QP5 -- The Overview while no channel is selected
            Dashboard {
                anchors.fill: parent
                visible: pane.selectedChannel < 0
            }
        }
    }
}
